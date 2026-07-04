use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use async_channel::Sender;
use codex_approval_service_api::ApprovalSessionCapability;
use config_service::Config;
use codex_config_types::Constrained;
use codex_config_types::McpServerTransportConfig;
use codex_config_types::OAuthCredentialsStoreMode;
use codex_connectors_api::AppInfo;
use codex_openai_files_api::OpenAiFileUploader;
use exec_server_api::ExecBackend;
use exec_server_api::ExecEnvironment;
use exec_server_api::HttpClient;
use http::HeaderMap;
use mcp_types::CodexAppsToolsCacheKey;
use mcp_types::EffectiveMcpServer;
use mcp_types::ElicitationResponse;
use mcp_types::ElicitationReviewRequest;
use mcp_types::ElicitationReviewerHandle;
use mcp_types::McpAuthStatusEntry;
use mcp_types::McpClientElicitationSupport;
use mcp_types::McpOAuthLoginSupport;
use mcp_types::McpServerElicitationRequestParams;
use mcp_types::McpToolApprovalMetadata;
use mcp_types::ResolvedMcpOAuthScopes;
use mcp_types::ToolPluginProvenance;
use plugin_service_api::PluginRuntime;
use protocol::mcp::CallToolResult;
use protocol::mcp::ListResourceTemplatesResult;
use protocol::mcp::ListResourcesResult;
use protocol::mcp::PaginatedRequestParams;
use protocol::mcp::ReadResourceRequestParams;
use protocol::mcp::ReadResourceResult;
use protocol::mcp::RequestId;
use protocol::mcp::Resource;
use protocol::mcp::ResourceTemplate;
use protocol::models::PermissionProfile;
use protocol::protocol::AskForApproval;
use protocol::protocol::Event;
use protocol::protocol::McpServerRefreshConfig;
use protocol::protocol::McpStartupFailure;
use skill_service_api::SkillMetadata;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadSessionCapability;
use thread_service_api::ThreadTurnCapability;
use tokio_util::sync::CancellationToken;
use tool_config::ToolsConfig;
use tool_service_api::DiscoverableTool;

pub type McpRuntimeFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type McpAuthFuture<'a, T> = McpRuntimeFuture<'a, T>;

#[derive(Clone, Debug)]
pub struct McpToolCallOutcome {
    pub result: CallToolResult,
    pub tool_input: serde_json::Value,
}

#[derive(Clone, Debug, Default)]
pub struct McpToolExposure {
    pub direct_tools: Vec<mcp_types::ToolInfo>,
    pub deferred_tools: Option<Vec<mcp_types::ToolInfo>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpAppUsageMetadata {
    pub connector_id: Option<String>,
    pub app_name: Option<String>,
}

/// Header-only auth provider used by MCP HTTP transports.
///
/// This intentionally carries only cheap, request-body-independent headers so
/// MCP runtime API consumers do not depend on the broader API provider/request
/// signing boundary.
pub trait McpAuthHeaderProvider: Send + Sync {
    fn to_auth_headers(&self) -> HeaderMap;
}

pub type SharedMcpAuthHeaderProvider = Arc<dyn McpAuthHeaderProvider>;

#[derive(Debug, Clone)]
pub struct StaticMcpAuthHeaderProvider {
    headers: HeaderMap,
}

impl StaticMcpAuthHeaderProvider {
    pub fn new(headers: HeaderMap) -> Self {
        Self { headers }
    }

    pub fn shared(headers: HeaderMap) -> SharedMcpAuthHeaderProvider {
        Arc::new(Self::new(headers))
    }
}

impl McpAuthHeaderProvider for StaticMcpAuthHeaderProvider {
    fn to_auth_headers(&self) -> HeaderMap {
        self.headers.clone()
    }
}

/// Owned request for an MCP OAuth login flow.
///
/// The request is intentionally host-neutral: it carries the server/login
/// values core needs, while the concrete runtime decides how to open browsers,
/// persist tokens, and handle provider-specific errors.
#[derive(Debug, Clone)]
pub struct McpOAuthLoginRequest {
    pub server_name: String,
    pub server_url: String,
    pub store_mode: OAuthCredentialsStoreMode,
    pub http_headers: Option<HashMap<String, String>>,
    pub env_http_headers: Option<HashMap<String, String>>,
    pub scopes: Vec<String>,
    pub oauth_client_id: Option<String>,
    pub oauth_resource: Option<String>,
    pub callback_port: Option<u16>,
    pub callback_url: Option<String>,
}

/// Host-provided MCP authentication capability used by session runtime code.
///
/// Implementations own network discovery, credential lookup and OAuth browser
/// flows. Consumers should depend on this trait instead of calling concrete
/// RMCP/OAuth helpers directly from session or connector orchestration code.
pub trait McpAuthRuntime: Send + Sync {
    fn oauth_login_support<'a>(
        &'a self,
        transport: &'a McpServerTransportConfig,
    ) -> McpAuthFuture<'a, McpOAuthLoginSupport>;

    fn perform_oauth_login<'a>(
        &'a self,
        request: McpOAuthLoginRequest,
    ) -> McpAuthFuture<'a, anyhow::Result<()>>;

    fn should_retry_without_scopes(
        &self,
        scopes: &ResolvedMcpOAuthScopes,
        error: &anyhow::Error,
    ) -> bool;

    fn compute_auth_statuses<'a>(
        &'a self,
        servers: Vec<(String, EffectiveMcpServer)>,
        store_mode: OAuthCredentialsStoreMode,
        host_owned_codex_apps_enabled: bool,
    ) -> McpAuthFuture<'a, HashMap<String, McpAuthStatusEntry>>;
}

/// Host-provided MCP tool-call capability used by session runtime code.
///
/// This trait intentionally covers only protocol-neutral tool execution and
/// server metadata. Broader manager operations that are also protocol-neutral
/// live on `McpConnectionRuntime`, so callers can depend on only the narrower
/// capability they need.
pub trait McpToolRuntime: Send + Sync {
    fn call_tool<'a>(
        &'a self,
        server: &'a str,
        tool: &'a str,
        arguments: Option<serde_json::Value>,
        meta: Option<serde_json::Value>,
    ) -> McpRuntimeFuture<'a, anyhow::Result<CallToolResult>>;

    fn server_origin(&self, server_name: &str) -> Option<String>;

    fn server_pollutes_memory(&self, server_name: &str) -> bool;

    fn is_host_owned_codex_apps_server(&self, server_name: &str) -> bool;

    fn server_supports_sandbox_state_meta_capability<'a>(
        &'a self,
        server_name: &'a str,
    ) -> McpRuntimeFuture<'a, anyhow::Result<bool>>;
}

/// Host-provided MCP connection capability for protocol-neutral manager calls.
///
/// Implementations own concrete RMCP clients, startup state, caches and process
/// shutdown. Session/runtime code should use these methods instead of directly
/// exposing RMCP request/result DTOs across crate boundaries.
pub trait McpConnectionRuntime: McpToolRuntime {
    fn has_servers(&self) -> bool;

    fn set_approval_policy(&self, approval_policy: &Constrained<AskForApproval>);

    fn set_permission_profile(&self, permission_profile: PermissionProfile);

    fn elicitations_auto_deny(&self) -> bool;

    fn set_elicitations_auto_deny(&self, auto_deny: bool);

    fn resolve_elicitation<'a>(
        &'a self,
        server_name: String,
        id: RequestId,
        response: ElicitationResponse,
    ) -> McpRuntimeFuture<'a, anyhow::Result<()>>;

    fn list_all_tools<'a>(&'a self) -> McpRuntimeFuture<'a, Vec<mcp_types::ToolInfo>>;

    fn hard_refresh_codex_apps_tools_cache<'a>(
        &'a self,
    ) -> McpRuntimeFuture<'a, anyhow::Result<Vec<mcp_types::ToolInfo>>>;

    fn wait_for_server_ready<'a>(
        &'a self,
        server_name: &'a str,
        timeout: Duration,
    ) -> McpRuntimeFuture<'a, bool>;

    fn required_startup_failures<'a>(
        &'a self,
        required_servers: &'a [String],
    ) -> McpRuntimeFuture<'a, Vec<McpStartupFailure>>;

    fn list_all_resources<'a>(&'a self) -> McpRuntimeFuture<'a, HashMap<String, Vec<Resource>>>;

    fn list_all_resource_templates<'a>(
        &'a self,
    ) -> McpRuntimeFuture<'a, HashMap<String, Vec<ResourceTemplate>>>;

    fn list_resources<'a>(
        &'a self,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> McpRuntimeFuture<'a, anyhow::Result<ListResourcesResult>>;

    fn list_resource_templates<'a>(
        &'a self,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> McpRuntimeFuture<'a, anyhow::Result<ListResourceTemplatesResult>>;

    fn read_resource<'a>(
        &'a self,
        server: &'a str,
        params: ReadResourceRequestParams,
    ) -> McpRuntimeFuture<'a, anyhow::Result<ReadResourceResult>>;

    fn shutdown<'a>(&'a mut self) -> McpRuntimeFuture<'a, ()>;
}

/// Owned startup request for constructing a concrete MCP connection runtime.
///
/// This mirrors the session/app-server inputs needed by the concrete RMCP
/// implementation while keeping the trait boundary free of RMCP DTOs.
pub struct McpConnectionRuntimeStartRequest {
    pub mcp_servers: HashMap<String, EffectiveMcpServer>,
    pub store_mode: OAuthCredentialsStoreMode,
    pub auth_entries: HashMap<String, McpAuthStatusEntry>,
    pub approval_policy: Constrained<AskForApproval>,
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

/// Concrete MCP connection runtime plus its startup cancellation handle.
pub struct McpConnectionRuntimeStart {
    pub runtime: Box<dyn McpConnectionRuntime>,
    pub startup_cancellation_token: CancellationToken,
}

/// Host-provided factory for MCP connection runtimes.
///
/// Core/session orchestration can depend on this trait while app-server, CLI,
/// TUI or test support decide which concrete RMCP implementation to inject.
pub trait McpConnectionRuntimeFactory: Send + Sync {
    fn uninitialized(
        &self,
        approval_policy: &Constrained<AskForApproval>,
        permission_profile: PermissionProfile,
    ) -> Box<dyn McpConnectionRuntime>;

    fn start(
        &self,
        request: McpConnectionRuntimeStartRequest,
    ) -> McpRuntimeFuture<'_, McpConnectionRuntimeStart>;
}

/// No-op MCP auth runtime for callers that intentionally do not start MCP.
#[derive(Debug, Default)]
pub struct DisabledMcpAuthRuntime;

impl McpAuthRuntime for DisabledMcpAuthRuntime {
    fn oauth_login_support<'a>(
        &'a self,
        _transport: &'a McpServerTransportConfig,
    ) -> McpAuthFuture<'a, McpOAuthLoginSupport> {
        Box::pin(async { McpOAuthLoginSupport::Unsupported })
    }

    fn perform_oauth_login<'a>(
        &'a self,
        request: McpOAuthLoginRequest,
    ) -> McpAuthFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            Err(anyhow::anyhow!(
                "MCP OAuth login is disabled for server '{}'",
                request.server_name
            ))
        })
    }

    fn should_retry_without_scopes(
        &self,
        _scopes: &ResolvedMcpOAuthScopes,
        _error: &anyhow::Error,
    ) -> bool {
        false
    }

    fn compute_auth_statuses<'a>(
        &'a self,
        _servers: Vec<(String, EffectiveMcpServer)>,
        _store_mode: OAuthCredentialsStoreMode,
        _host_owned_codex_apps_enabled: bool,
    ) -> McpAuthFuture<'a, HashMap<String, McpAuthStatusEntry>> {
        Box::pin(async { HashMap::new() })
    }
}

/// No-op MCP connection runtime used by composition-light callers.
#[derive(Debug, Default)]
pub struct DisabledMcpConnectionRuntime {
    elicitations_auto_deny: AtomicBool,
}

impl McpToolRuntime for DisabledMcpConnectionRuntime {
    fn call_tool<'a>(
        &'a self,
        server: &'a str,
        tool: &'a str,
        _arguments: Option<serde_json::Value>,
        _meta: Option<serde_json::Value>,
    ) -> McpRuntimeFuture<'a, anyhow::Result<CallToolResult>> {
        Box::pin(async move {
            Err(anyhow::anyhow!(
                "MCP runtime is disabled; cannot call tool '{server}/{tool}'"
            ))
        })
    }

    fn server_origin(&self, _server_name: &str) -> Option<String> {
        None
    }

    fn server_pollutes_memory(&self, _server_name: &str) -> bool {
        false
    }

    fn is_host_owned_codex_apps_server(&self, _server_name: &str) -> bool {
        false
    }

    fn server_supports_sandbox_state_meta_capability<'a>(
        &'a self,
        _server_name: &'a str,
    ) -> McpRuntimeFuture<'a, anyhow::Result<bool>> {
        Box::pin(async { Ok(false) })
    }
}

impl McpConnectionRuntime for DisabledMcpConnectionRuntime {
    fn has_servers(&self) -> bool {
        false
    }

    fn set_approval_policy(&self, _approval_policy: &Constrained<AskForApproval>) {}

    fn set_permission_profile(&self, _permission_profile: PermissionProfile) {}

    fn elicitations_auto_deny(&self) -> bool {
        self.elicitations_auto_deny.load(Ordering::SeqCst)
    }

    fn set_elicitations_auto_deny(&self, auto_deny: bool) {
        self.elicitations_auto_deny
            .store(auto_deny, Ordering::SeqCst);
    }

    fn resolve_elicitation<'a>(
        &'a self,
        server_name: String,
        _id: RequestId,
        _response: ElicitationResponse,
    ) -> McpRuntimeFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            Err(anyhow::anyhow!(
                "MCP runtime is disabled; cannot resolve elicitation for server '{server_name}'"
            ))
        })
    }

    fn list_all_tools<'a>(&'a self) -> McpRuntimeFuture<'a, Vec<mcp_types::ToolInfo>> {
        Box::pin(async { Vec::new() })
    }

    fn hard_refresh_codex_apps_tools_cache<'a>(
        &'a self,
    ) -> McpRuntimeFuture<'a, anyhow::Result<Vec<mcp_types::ToolInfo>>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn wait_for_server_ready<'a>(
        &'a self,
        _server_name: &'a str,
        _timeout: Duration,
    ) -> McpRuntimeFuture<'a, bool> {
        Box::pin(async { false })
    }

    fn required_startup_failures<'a>(
        &'a self,
        _required_servers: &'a [String],
    ) -> McpRuntimeFuture<'a, Vec<McpStartupFailure>> {
        Box::pin(async { Vec::new() })
    }

    fn list_all_resources<'a>(&'a self) -> McpRuntimeFuture<'a, HashMap<String, Vec<Resource>>> {
        Box::pin(async { HashMap::new() })
    }

    fn list_all_resource_templates<'a>(
        &'a self,
    ) -> McpRuntimeFuture<'a, HashMap<String, Vec<ResourceTemplate>>> {
        Box::pin(async { HashMap::new() })
    }

    fn list_resources<'a>(
        &'a self,
        server: &'a str,
        _params: Option<PaginatedRequestParams>,
    ) -> McpRuntimeFuture<'a, anyhow::Result<ListResourcesResult>> {
        Box::pin(async move {
            Err(anyhow::anyhow!(
                "MCP runtime is disabled; cannot list resources for server '{server}'"
            ))
        })
    }

    fn list_resource_templates<'a>(
        &'a self,
        server: &'a str,
        _params: Option<PaginatedRequestParams>,
    ) -> McpRuntimeFuture<'a, anyhow::Result<ListResourceTemplatesResult>> {
        Box::pin(async move {
            Err(anyhow::anyhow!(
                "MCP runtime is disabled; cannot list resource templates for server '{server}'"
            ))
        })
    }

    fn read_resource<'a>(
        &'a self,
        server: &'a str,
        _params: ReadResourceRequestParams,
    ) -> McpRuntimeFuture<'a, anyhow::Result<ReadResourceResult>> {
        Box::pin(async move {
            Err(anyhow::anyhow!(
                "MCP runtime is disabled; cannot read resource for server '{server}'"
            ))
        })
    }

    fn shutdown<'a>(&'a mut self) -> McpRuntimeFuture<'a, ()> {
        Box::pin(async {})
    }
}

/// Factory that always returns disabled MCP connection runtimes.
#[derive(Debug, Default)]
pub struct DisabledMcpConnectionRuntimeFactory;

impl McpConnectionRuntimeFactory for DisabledMcpConnectionRuntimeFactory {
    fn uninitialized(
        &self,
        _approval_policy: &Constrained<AskForApproval>,
        _permission_profile: PermissionProfile,
    ) -> Box<dyn McpConnectionRuntime> {
        Box::new(DisabledMcpConnectionRuntime::default())
    }

    fn start(
        &self,
        _request: McpConnectionRuntimeStartRequest,
    ) -> McpRuntimeFuture<'_, McpConnectionRuntimeStart> {
        Box::pin(async {
            McpConnectionRuntimeStart {
                runtime: Box::new(DisabledMcpConnectionRuntime::default()),
                startup_cancellation_token: CancellationToken::new(),
            }
        })
    }
}

/// Runtime placement information used when starting MCP server transports.
///
/// `McpConfig` describes which servers exist. This value describes where those
/// servers should run for the current caller. Keep it explicit at manager
/// construction time so status/snapshot paths and real sessions make the same
/// local-vs-remote decision. `fallback_cwd` is not a per-server override; it is
/// used when a stdio server omits `cwd` and the launcher needs a concrete
/// process working directory.
#[derive(Clone)]
pub struct McpRuntimeEnvironment {
    remote_available: bool,
    remote_exec_backend: Arc<dyn ExecBackend>,
    local_http_client: Arc<dyn HttpClient>,
    remote_http_client: Arc<dyn HttpClient>,
    fallback_cwd: PathBuf,
}

pub struct McpRuntimeEnvironmentParams {
    pub remote_available: bool,
    pub remote_exec_backend: Arc<dyn ExecBackend>,
    pub local_http_client: Arc<dyn HttpClient>,
    pub remote_http_client: Arc<dyn HttpClient>,
    pub fallback_cwd: PathBuf,
}

impl McpRuntimeEnvironment {
    pub fn new(params: McpRuntimeEnvironmentParams) -> Self {
        let McpRuntimeEnvironmentParams {
            remote_available,
            remote_exec_backend,
            local_http_client,
            remote_http_client,
            fallback_cwd,
        } = params;
        Self {
            remote_available,
            remote_exec_backend,
            local_http_client,
            remote_http_client,
            fallback_cwd,
        }
    }

    pub fn remote_available(&self) -> bool {
        self.remote_available
    }

    pub fn remote_exec_backend(&self) -> Arc<dyn ExecBackend> {
        Arc::clone(&self.remote_exec_backend)
    }

    pub fn http_client(&self, remote: bool) -> Arc<dyn HttpClient> {
        if remote {
            Arc::clone(&self.remote_http_client)
        } else {
            Arc::clone(&self.local_http_client)
        }
    }

    pub fn fallback_cwd(&self) -> PathBuf {
        self.fallback_cwd.clone()
    }
}

/// Global MCP service API consumed by tool-service.
pub trait McpServiceApi: Send + Sync + 'static {
    fn list_accessible_connectors(
        &self,
        all_mcp_tools: &[mcp_types::ToolInfo],
        config: &Config,
    ) -> Vec<AppInfo>;

    fn list_available_connectors<'a>(
        &self,
        plugin_runtime: &'a dyn PluginRuntime,
        all_mcp_tools: &'a [mcp_types::ToolInfo],
        config: &'a Config,
    ) -> McpRuntimeFuture<'a, Vec<AppInfo>>;

    fn list_discoverable_tools<'a>(
        &self,
        turn: &'a dyn ThreadTurnCapability,
        plugin_runtime: &'a dyn PluginRuntime,
        accessible_connectors: &'a [AppInfo],
        config: &'a Config,
        app_server_client_name: Option<&'a str>,
        tool_suggest_enabled: bool,
        apps_enabled: bool,
    ) -> McpRuntimeFuture<'a, Result<Vec<DiscoverableTool>, String>>;

    fn build_tool_exposure(
        &self,
        all_mcp_tools: &[mcp_types::ToolInfo],
        connectors: Option<&[AppInfo]>,
        explicitly_enabled_connectors: &[AppInfo],
        config: &Config,
        tools_config: &ToolsConfig,
    ) -> McpToolExposure;

    fn maybe_prompt_and_install_mcp_dependencies<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        config: &'a Config,
        cancellation_token: &'a CancellationToken,
        mentioned_skills: &'a [SkillMetadata],
        elicitation_reviewer: Option<ElicitationReviewerHandle>,
    ) -> McpRuntimeFuture<'a, ()>;

    fn lookup_app_usage_metadata(
        &self,
        all_mcp_tools: &[mcp_types::ToolInfo],
        server: &str,
        tool_name: &str,
    ) -> Option<McpAppUsageMetadata>;

    fn configured_servers<'a>(
        &self,
        plugin_runtime: &'a dyn PluginRuntime,
        config: &'a Config,
    ) -> McpRuntimeFuture<'a, HashMap<String, config_service::McpServerConfig>>;

    fn effective_servers<'a>(
        &self,
        plugin_runtime: &'a dyn PluginRuntime,
        config: &'a Config,
        auth_context: Option<&'a mcp_types::CodexAppsAuthContext>,
    ) -> McpRuntimeFuture<'a, HashMap<String, EffectiveMcpServer>>;

    fn tool_plugin_provenance<'a>(
        &self,
        plugin_runtime: &'a dyn PluginRuntime,
        config: &'a Config,
    ) -> McpRuntimeFuture<'a, ToolPluginProvenance>;

    fn list_accessible_and_enabled_connectors(
        &self,
        all_mcp_tools: &[mcp_types::ToolInfo],
        config: &Config,
    ) -> Vec<AppInfo>;

    fn fetch_accessible_connectors<'a>(
        &self,
        plugin_runtime: &'a dyn PluginRuntime,
        config: &'a Config,
        auth_snapshot: Option<&'a codex_auth_types::RequestAuthSnapshot>,
        environment_provider: &'a dyn exec_server_api::ExecEnvironmentProvider,
        mcp_auth_runtime: &'a dyn McpAuthRuntime,
        mcp_connection_runtime_factory: &'a dyn McpConnectionRuntimeFactory,
    ) -> McpRuntimeFuture<'a, anyhow::Result<Vec<AppInfo>>>;

    fn app_tool_policy(
        &self,
        config: &Config,
        metadata: Option<&mcp_types::McpToolApprovalMetadata>,
        tool_name: &str,
    ) -> thread_service_api::ThreadAppToolPolicy;

    fn list_cached_accessible_connectors<'a>(
        &self,
        config: &'a Config,
        auth_snapshot: Option<&'a codex_auth_types::RequestAuthSnapshot>,
    ) -> McpRuntimeFuture<'a, Option<Vec<AppInfo>>>;

    fn refresh_accessible_connectors_cache(
        &self,
        config: &Config,
        connector_auth_context: Option<&mcp_types::CodexAppsAuthContext>,
        mcp_tools: &[mcp_types::ToolInfo],
    );

    fn codex_apps_auth_context(
        &self,
        auth: Option<&codex_auth_types::RequestAuthSnapshot>,
    ) -> Option<mcp_types::CodexAppsAuthContext>;

    fn codex_apps_auth_provider(
        &self,
        auth: Option<&codex_auth_types::RequestAuthSnapshot>,
    ) -> Option<SharedMcpAuthHeaderProvider>;

    fn build_runtime_environment(
        &self,
        environment: Arc<dyn ExecEnvironment>,
        local_environment: Arc<dyn ExecEnvironment>,
        fallback_cwd: PathBuf,
    ) -> McpRuntimeEnvironment;

    fn start_connection_runtime<'a>(
        &self,
        factory: &'a dyn McpConnectionRuntimeFactory,
        request: McpConnectionRuntimeStartRequest,
    ) -> McpRuntimeFuture<'a, McpConnectionRuntimeStart>;

    fn review_guardian_elicitation<'a>(
        &self,
        session: Arc<dyn ApprovalSessionCapability>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        request: ElicitationReviewRequest,
    ) -> McpRuntimeFuture<'a, anyhow::Result<Option<ElicitationResponse>>>;

    fn rewrite_tool_arguments_for_openai_files<'a>(
        &self,
        uploader: &'a dyn OpenAiFileUploader,
        auth: Option<&'a codex_auth_types::RequestAuthSnapshot>,
        chatgpt_base_url: &'a str,
        turn: &'a dyn ThreadRuntimeCapability,
        arguments_value: Option<serde_json::Value>,
        openai_file_input_params: Option<&'a [String]>,
    ) -> McpRuntimeFuture<'a, Result<Option<serde_json::Value>, String>>;

    fn custom_tool_approval_mode<'a>(
        &self,
        plugin_runtime: &'a dyn PluginRuntime,
        config: &'a Config,
        server: &'a str,
        tool_name: &'a str,
    ) -> McpRuntimeFuture<'a, codex_config_types::AppToolApproval>;

    fn persist_codex_app_tool_approval<'a>(
        &self,
        config: &'a Config,
        connector_id: &'a str,
        tool_name: &'a str,
    ) -> McpRuntimeFuture<'a, anyhow::Result<()>>;

    fn persist_non_app_mcp_tool_approval<'a>(
        &self,
        plugin_runtime: &'a dyn PluginRuntime,
        config: &'a Config,
        server: &'a str,
        tool_name: &'a str,
    ) -> McpRuntimeFuture<'a, anyhow::Result<()>>;

    fn request_server_elicitation<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        request_id: RequestId,
        params: McpServerElicitationRequestParams,
    ) -> McpRuntimeFuture<'a, Option<ElicitationResponse>>;

    fn resolve_elicitation<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
        server_name: String,
        request_id: RequestId,
        response: ElicitationResponse,
    ) -> McpRuntimeFuture<'a, Result<(), String>>;

    fn refresh_servers_if_requested<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        elicitation_reviewer: Option<ElicitationReviewerHandle>,
    ) -> McpRuntimeFuture<'a, ()>;

    fn queue_server_refresh<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
        refresh_config: McpServerRefreshConfig,
    ) -> McpRuntimeFuture<'a, ()>;

    fn refresh_servers_now<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        refresh_config: McpServerRefreshConfig,
        elicitation_reviewer: Option<ElicitationReviewerHandle>,
    ) -> McpRuntimeFuture<'a, ()>;

    fn cancel_startup<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
    ) -> McpRuntimeFuture<'a, ()>;

    fn hard_refresh_codex_apps_tools_cache<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
    ) -> McpRuntimeFuture<'a, Result<Vec<mcp_types::ToolInfo>, String>>;

    fn lookup_tool_metadata<'a>(
        &self,
        session: Arc<dyn ThreadSessionCapability>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        server: &'a str,
        tool_name: &'a str,
    ) -> McpRuntimeFuture<'a, Option<McpToolApprovalMetadata>>;

    fn call_tool<'a>(
        &self,
        session: Arc<dyn ThreadSessionCapability>,
        approval_session: Arc<dyn ApprovalSessionCapability>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        call_id: String,
        server: String,
        tool_name: String,
        hook_tool_name: String,
        arguments: String,
    ) -> McpRuntimeFuture<'a, McpToolCallOutcome>;

    fn list_resources<'a>(
        &self,
        session: Arc<dyn ThreadSessionCapability>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        call_id: String,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> McpRuntimeFuture<'a, Result<ListResourcesResult, String>>;

    fn list_all_resources<'a>(
        &self,
        session: Arc<dyn ThreadSessionCapability>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        call_id: String,
    ) -> McpRuntimeFuture<'a, HashMap<String, Vec<Resource>>>;

    fn list_resource_templates<'a>(
        &self,
        session: Arc<dyn ThreadSessionCapability>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        call_id: String,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> McpRuntimeFuture<'a, Result<ListResourceTemplatesResult, String>>;

    fn list_all_resource_templates<'a>(
        &self,
        session: Arc<dyn ThreadSessionCapability>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        call_id: String,
    ) -> McpRuntimeFuture<'a, HashMap<String, Vec<ResourceTemplate>>>;

    fn read_resource<'a>(
        &self,
        session: Arc<dyn ThreadSessionCapability>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        call_id: String,
        server: &'a str,
        params: ReadResourceRequestParams,
    ) -> McpRuntimeFuture<'a, Result<ReadResourceResult, String>>;
}
