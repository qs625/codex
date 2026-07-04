use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::time::Duration;

use codex_config_types::Constrained;
use codex_config_types::McpServerConfig;
use codex_config_types::McpServerTransportConfig;
use codex_config_types::OAuthCredentialsStoreMode;
use plugin_service_api::PluginCapabilitySummary;
use protocol::protocol::AskForApproval;

use crate::CODEX_APPS_MCP_SERVER_NAME;
use crate::CodexAppsAuthContext;
use crate::EffectiveMcpServer;
use crate::McpClientElicitationSupport;
use crate::ToolPluginProvenance;

const CODEX_CONNECTORS_TOKEN_ENV_VAR: &str = "CODEX_CONNECTORS_TOKEN";

/// MCP runtime settings derived from `codex_core::config::Config`.
///
/// This struct contains long-lived configuration values needed to compute the
/// effective MCP server view and to initialize the MCP runtime. Request-scoped
/// or auth-scoped state should be passed explicitly to helper functions.
#[derive(Debug, Clone)]
pub struct McpConfig {
    /// Base URL for ChatGPT-hosted app MCP servers, copied from the root config.
    pub chatgpt_base_url: String,
    /// Optional path override for the host-owned apps MCP server.
    pub apps_mcp_path_override: Option<String>,
    /// Codex home directory used for MCP OAuth state and app-tool cache files.
    pub codex_home: PathBuf,
    /// Preferred credential store for MCP OAuth tokens.
    pub mcp_oauth_credentials_store_mode: OAuthCredentialsStoreMode,
    /// Optional fixed localhost callback port for MCP OAuth login.
    pub mcp_oauth_callback_port: Option<u16>,
    /// Optional OAuth redirect URI override for MCP login.
    pub mcp_oauth_callback_url: Option<String>,
    /// Whether skill MCP dependency installation prompts are enabled.
    pub skill_mcp_dependency_install_enabled: bool,
    /// Approval policy used for MCP tool calls and MCP elicitation requests.
    pub approval_policy: Constrained<AskForApproval>,
    /// Optional path to `codex-linux-sandbox` for sandboxed MCP tool execution.
    pub codex_linux_sandbox_exe: Option<PathBuf>,
    /// Whether to use legacy Landlock behavior in the MCP sandbox state.
    pub use_legacy_landlock: bool,
    /// Whether the app MCP integration is enabled by config.
    ///
    /// ChatGPT auth is checked separately at runtime before the host-owned apps
    /// MCP server is added.
    pub apps_enabled: bool,
    /// Client-side elicitation support advertised during MCP initialization.
    pub client_elicitation_support: McpClientElicitationSupport,
    /// Config-backed MCP servers keyed by server name.
    ///
    /// Runtime-only additions are merged later by [`effective_mcp_servers`].
    pub configured_mcp_servers: HashMap<String, McpServerConfig>,
    /// Plugin metadata used to attribute MCP tools/connectors to plugin display names.
    pub plugin_capability_summaries: Vec<PluginCapabilitySummary>,
}

pub fn with_codex_apps_mcp(
    mut servers: HashMap<String, EffectiveMcpServer>,
    auth_context: Option<&CodexAppsAuthContext>,
    config: &McpConfig,
) -> HashMap<String, EffectiveMcpServer> {
    if host_owned_codex_apps_enabled(config, auth_context) {
        servers.insert(
            CODEX_APPS_MCP_SERVER_NAME.to_string(),
            EffectiveMcpServer::configured(codex_apps_mcp_server_config(config)),
        );
    } else {
        servers.remove(CODEX_APPS_MCP_SERVER_NAME);
    }
    servers
}

pub fn host_owned_codex_apps_enabled(
    config: &McpConfig,
    auth_context: Option<&CodexAppsAuthContext>,
) -> bool {
    config.apps_enabled && auth_context.is_some_and(|auth_context| auth_context.uses_codex_backend)
}

pub fn configured_mcp_servers(config: &McpConfig) -> HashMap<String, McpServerConfig> {
    config.configured_mcp_servers.clone()
}

pub fn effective_mcp_servers(
    config: &McpConfig,
    auth_context: Option<&CodexAppsAuthContext>,
) -> HashMap<String, EffectiveMcpServer> {
    effective_mcp_servers_from_configured(configured_mcp_servers(config), config, auth_context)
}

pub fn effective_mcp_servers_from_configured(
    configured_servers: HashMap<String, McpServerConfig>,
    config: &McpConfig,
    auth_context: Option<&CodexAppsAuthContext>,
) -> HashMap<String, EffectiveMcpServer> {
    let servers = configured_servers
        .into_iter()
        .map(|(name, server)| (name, EffectiveMcpServer::configured(server)))
        .collect::<HashMap<_, _>>();
    with_codex_apps_mcp(servers, auth_context, config)
}

pub fn tool_plugin_provenance(config: &McpConfig) -> ToolPluginProvenance {
    tool_plugin_provenance_from_capability_summaries(&config.plugin_capability_summaries)
}

fn tool_plugin_provenance_from_capability_summaries(
    capability_summaries: &[PluginCapabilitySummary],
) -> ToolPluginProvenance {
    let connector_sources = capability_summaries.iter().flat_map(|plugin| {
        plugin
            .app_connector_ids
            .iter()
            .map(|connector_id| (connector_id.0.clone(), plugin.display_name.clone()))
    });
    let mcp_server_sources = capability_summaries.iter().flat_map(|plugin| {
        plugin
            .mcp_server_names
            .iter()
            .map(|server_name| (server_name.clone(), plugin.display_name.clone()))
    });
    ToolPluginProvenance::from_plugin_sources(connector_sources, mcp_server_sources)
}

fn codex_apps_mcp_url(config: &McpConfig) -> String {
    codex_apps_mcp_url_for_base_url(
        &config.chatgpt_base_url,
        config.apps_mcp_path_override.as_deref(),
    )
}

fn codex_apps_mcp_bearer_token_env_var() -> Option<String> {
    match env::var(CODEX_CONNECTORS_TOKEN_ENV_VAR) {
        Ok(value) if !value.trim().is_empty() => Some(CODEX_CONNECTORS_TOKEN_ENV_VAR.to_string()),
        Ok(_) => None,
        Err(env::VarError::NotPresent) => None,
        Err(env::VarError::NotUnicode(_)) => Some(CODEX_CONNECTORS_TOKEN_ENV_VAR.to_string()),
    }
}

fn normalize_codex_apps_base_url(base_url: &str) -> String {
    let mut base_url = base_url.trim_end_matches('/').to_string();
    if (base_url.starts_with("https://chatgpt.com")
        || base_url.starts_with("https://chat.openai.com"))
        && !base_url.contains("/backend-api")
    {
        base_url = format!("{base_url}/backend-api");
    }
    base_url
}

fn codex_apps_mcp_url_for_base_url(base_url: &str, apps_mcp_path_override: Option<&str>) -> String {
    let base_url = normalize_codex_apps_base_url(base_url);
    let (base_url, default_path) = if base_url.contains("/backend-api") {
        (base_url, "wham/apps")
    } else if base_url.contains("/api/codex") {
        (base_url, "apps")
    } else {
        (format!("{base_url}/api/codex"), "apps")
    };
    let path = apps_mcp_path_override
        .unwrap_or(default_path)
        .trim_start_matches('/');
    format!("{base_url}/{path}")
}

fn codex_apps_mcp_server_config(config: &McpConfig) -> McpServerConfig {
    let url = codex_apps_mcp_url(config);

    McpServerConfig {
        transport: McpServerTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var: codex_apps_mcp_bearer_token_env_var(),
            http_headers: None,
            env_http_headers: None,
        },
        experimental_environment: None,
        enabled: true,
        required: false,
        supports_parallel_tool_calls: false,
        disabled_reason: None,
        startup_timeout_sec: Some(Duration::from_secs(30)),
        tool_timeout_sec: None,
        default_tools_approval_mode: None,
        enabled_tools: None,
        disabled_tools: None,
        scopes: None,
        oauth: None,
        oauth_resource: None,
        tools: HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_config_types::Constrained;
    use plugin_service_api::AppConnectorId;

    fn test_mcp_config(codex_home: PathBuf) -> McpConfig {
        McpConfig {
            chatgpt_base_url: "https://chatgpt.com".to_string(),
            apps_mcp_path_override: None,
            codex_home,
            mcp_oauth_credentials_store_mode: OAuthCredentialsStoreMode::default(),
            mcp_oauth_callback_port: None,
            mcp_oauth_callback_url: None,
            skill_mcp_dependency_install_enabled: true,
            approval_policy: Constrained::allow_any(AskForApproval::OnFailure),
            codex_linux_sandbox_exe: None,
            use_legacy_landlock: false,
            apps_enabled: false,
            client_elicitation_support: McpClientElicitationSupport::Disabled,
            configured_mcp_servers: HashMap::new(),
            plugin_capability_summaries: Vec::new(),
        }
    }

    fn test_codex_apps_auth_context() -> CodexAppsAuthContext {
        CodexAppsAuthContext {
            uses_codex_backend: true,
            account_id: Some("acct_test".to_string()),
            chatgpt_user_id: Some("user_test".to_string()),
            is_workspace_account: false,
        }
    }

    #[test]
    fn tool_plugin_provenance_collects_app_and_mcp_sources() {
        let provenance = tool_plugin_provenance_from_capability_summaries(&[
            PluginCapabilitySummary {
                display_name: "alpha-plugin".to_string(),
                app_connector_ids: vec![AppConnectorId("connector_example".to_string())],
                mcp_server_names: vec!["alpha".to_string()],
                ..PluginCapabilitySummary::default()
            },
            PluginCapabilitySummary {
                display_name: "beta-plugin".to_string(),
                app_connector_ids: vec![
                    AppConnectorId("connector_example".to_string()),
                    AppConnectorId("connector_gmail".to_string()),
                ],
                mcp_server_names: vec!["beta".to_string()],
                ..PluginCapabilitySummary::default()
            },
        ]);

        assert_eq!(
            provenance.plugin_display_names_for_connector_id("connector_example"),
            &["alpha-plugin".to_string(), "beta-plugin".to_string()]
        );
        assert_eq!(
            provenance.plugin_display_names_for_connector_id("connector_gmail"),
            &["beta-plugin".to_string()]
        );
        assert_eq!(
            provenance.plugin_display_names_for_mcp_server_name("alpha"),
            &["alpha-plugin".to_string()]
        );
        assert_eq!(
            provenance.plugin_display_names_for_mcp_server_name("beta"),
            &["beta-plugin".to_string()]
        );
    }

    #[test]
    fn codex_apps_mcp_url_for_base_url_keeps_existing_paths() {
        assert_eq!(
            codex_apps_mcp_url_for_base_url(
                "https://chatgpt.com/backend-api",
                /*apps_mcp_path_override*/ None,
            ),
            "https://chatgpt.com/backend-api/wham/apps"
        );
        assert_eq!(
            codex_apps_mcp_url_for_base_url(
                "https://chat.openai.com",
                /*apps_mcp_path_override*/ None,
            ),
            "https://chat.openai.com/backend-api/wham/apps"
        );
        assert_eq!(
            codex_apps_mcp_url_for_base_url(
                "http://localhost:8080/api/codex",
                /*apps_mcp_path_override*/ None,
            ),
            "http://localhost:8080/api/codex/apps"
        );
        assert_eq!(
            codex_apps_mcp_url_for_base_url(
                "http://localhost:8080",
                /*apps_mcp_path_override*/ None,
            ),
            "http://localhost:8080/api/codex/apps"
        );
    }

    #[test]
    fn codex_apps_mcp_url_uses_legacy_codex_apps_path() {
        let config = test_mcp_config(PathBuf::from("/tmp"));

        assert_eq!(
            codex_apps_mcp_url(&config),
            "https://chatgpt.com/backend-api/wham/apps"
        );
    }

    #[test]
    fn codex_apps_server_config_uses_legacy_codex_apps_path() {
        let mut config = test_mcp_config(PathBuf::from("/tmp"));
        let auth_context = test_codex_apps_auth_context();

        let mut servers = with_codex_apps_mcp(HashMap::new(), /*auth_context*/ None, &config);
        assert!(!servers.contains_key(CODEX_APPS_MCP_SERVER_NAME));

        config.apps_enabled = true;

        servers = with_codex_apps_mcp(servers, Some(&auth_context), &config);
        let server = servers
            .get(CODEX_APPS_MCP_SERVER_NAME)
            .expect("codex apps should be present when apps is enabled");
        let config = server
            .configured_config()
            .expect("codex apps should use configured transport");
        let url = match &config.transport {
            McpServerTransportConfig::StreamableHttp { url, .. } => url,
            _ => panic!("expected streamable http transport for codex apps"),
        };

        assert_eq!(url, "https://chatgpt.com/backend-api/wham/apps");
    }

    #[test]
    fn codex_apps_server_config_uses_configured_apps_mcp_path_override() {
        let mut config = test_mcp_config(PathBuf::from("/tmp"));
        config.apps_mcp_path_override = Some("/custom/mcp".to_string());
        config.apps_enabled = true;
        let auth_context = test_codex_apps_auth_context();

        let servers = with_codex_apps_mcp(HashMap::new(), Some(&auth_context), &config);
        let server = servers
            .get(CODEX_APPS_MCP_SERVER_NAME)
            .expect("codex apps should be present when apps is enabled");
        let config = server
            .configured_config()
            .expect("codex apps should use configured transport");
        let url = match &config.transport {
            McpServerTransportConfig::StreamableHttp { url, .. } => url,
            _ => panic!("expected streamable http transport for codex apps"),
        };

        assert_eq!(url, "https://chatgpt.com/backend-api/custom/mcp");
    }

    #[test]
    fn effective_mcp_servers_preserve_user_servers_and_add_codex_apps() {
        let mut config = test_mcp_config(PathBuf::from("/tmp"));
        config.apps_enabled = true;
        let auth_context = test_codex_apps_auth_context();

        config.configured_mcp_servers.insert(
            "sample".to_string(),
            McpServerConfig {
                transport: McpServerTransportConfig::StreamableHttp {
                    url: "https://user.example/mcp".to_string(),
                    bearer_token_env_var: None,
                    http_headers: None,
                    env_http_headers: None,
                },
                experimental_environment: None,
                enabled: true,
                required: false,
                supports_parallel_tool_calls: false,
                disabled_reason: None,
                startup_timeout_sec: None,
                tool_timeout_sec: None,
                default_tools_approval_mode: None,
                enabled_tools: None,
                disabled_tools: None,
                scopes: None,
                oauth: None,
                oauth_resource: None,
                tools: HashMap::new(),
            },
        );
        config.configured_mcp_servers.insert(
            "docs".to_string(),
            McpServerConfig {
                transport: McpServerTransportConfig::StreamableHttp {
                    url: "https://docs.example/mcp".to_string(),
                    bearer_token_env_var: None,
                    http_headers: None,
                    env_http_headers: None,
                },
                experimental_environment: None,
                enabled: true,
                required: false,
                supports_parallel_tool_calls: false,
                disabled_reason: None,
                startup_timeout_sec: None,
                tool_timeout_sec: None,
                default_tools_approval_mode: None,
                enabled_tools: None,
                disabled_tools: None,
                scopes: None,
                oauth: None,
                oauth_resource: None,
                tools: HashMap::new(),
            },
        );

        let effective = effective_mcp_servers(&config, Some(&auth_context));

        let sample = effective.get("sample").expect("user server should exist");
        let docs = effective
            .get("docs")
            .expect("configured server should exist");
        let codex_apps = effective
            .get(CODEX_APPS_MCP_SERVER_NAME)
            .expect("codex apps server should exist");

        let sample = sample
            .configured_config()
            .expect("configured server should retain transport");
        let docs = docs
            .configured_config()
            .expect("configured server should retain transport");
        let codex_apps = codex_apps
            .configured_config()
            .expect("codex apps should use configured transport");

        match &sample.transport {
            McpServerTransportConfig::StreamableHttp { url, .. } => {
                assert_eq!(url, "https://user.example/mcp");
            }
            other => panic!("expected streamable http transport, got {other:?}"),
        }
        match &docs.transport {
            McpServerTransportConfig::StreamableHttp { url, .. } => {
                assert_eq!(url, "https://docs.example/mcp");
            }
            other => panic!("expected streamable http transport, got {other:?}"),
        }
        match &codex_apps.transport {
            McpServerTransportConfig::StreamableHttp { url, .. } => {
                assert_eq!(url, "https://chatgpt.com/backend-api/wham/apps");
            }
            other => panic!("expected streamable http transport, got {other:?}"),
        }
    }
}
