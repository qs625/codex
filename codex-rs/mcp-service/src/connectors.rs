use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use std::time::Instant;

use async_channel::unbounded;
use codex_auth_types::RequestAuthSnapshot;
use transport_client_identity::originator;
use config_service::Config;
use codex_config_types::ToolSuggestDiscoverableType;
use codex_connectors_api::AppInfo;
use codex_connectors_api::ConnectorDirectoryCacheContext;
use codex_connectors_api::ConnectorDirectoryCacheKey;
use codex_features::Feature;
use exec_server_api::ExecEnvironmentProvider;
use mcp_service_api::McpAuthRuntime;
use mcp_service_api::McpConnectionRuntime;
use mcp_service_api::McpConnectionRuntimeFactory;
use mcp_service_api::McpConnectionRuntimeStartRequest;
use mcp_types::CODEX_APPS_MCP_SERVER_NAME;
use mcp_types::CodexAppsAuthContext;
use mcp_types::ToolInfo;
use mcp_types::ToolPluginProvenance;
use mcp_types::codex_apps_tools_cache_key;
use mcp_types::host_owned_codex_apps_enabled;
use mcp_types::tool_plugin_provenance;
use mcp_types::with_codex_apps_mcp;
use plugin_service_api::PluginRuntime;
use protocol::models::PermissionProfile;
use tool_service_api::DiscoverablePluginInfo;
use tool_service_api::DiscoverableTool;
use tracing::warn;

use crate::codex_apps_auth_context;
use crate::codex_apps_auth_provider;
use crate::mcp_runtime_environment;

const CONNECTORS_READY_TIMEOUT_ON_EMPTY_TOOLS: Duration = Duration::from_secs(30);
const CONNECTORS_STARTUP_SUBMIT_ID: &str = "";

#[derive(Clone, PartialEq, Eq)]
struct AccessibleConnectorsCacheKey {
    chatgpt_base_url: String,
    account_id: Option<String>,
    chatgpt_user_id: Option<String>,
    is_workspace_account: bool,
}

#[derive(Clone)]
struct CachedAccessibleConnectors {
    key: AccessibleConnectorsCacheKey,
    expires_at: Instant,
    connectors: Vec<AppInfo>,
}

static ACCESSIBLE_CONNECTORS_CACHE: LazyLock<StdMutex<Option<CachedAccessibleConnectors>>> =
    LazyLock::new(|| StdMutex::new(None));

#[derive(Debug, Clone)]
pub struct AccessibleConnectorsStatus {
    pub connectors: Vec<AppInfo>,
    pub codex_apps_ready: bool,
}

pub async fn list_accessible_connectors_from_mcp_tools(
    config: &Config,
    auth_snapshot: Option<&RequestAuthSnapshot>,
    plugin_runtime: &dyn plugin_service_api::PluginRuntime,
    environment_provider: &dyn ExecEnvironmentProvider,
    mcp_auth_runtime: &dyn McpAuthRuntime,
    mcp_connection_runtime_factory: &dyn McpConnectionRuntimeFactory,
) -> anyhow::Result<Vec<AppInfo>> {
    Ok(
        list_accessible_connectors_from_mcp_tools_with_options_and_status(
            config,
            auth_snapshot,
            /*force_refetch*/ false,
            plugin_runtime,
            environment_provider,
            mcp_auth_runtime,
            mcp_connection_runtime_factory,
        )
        .await?
        .connectors,
    )
}

pub async fn list_tool_suggest_discoverable_tools_with_auth(
    config: &Config,
    plugin_runtime: &dyn PluginRuntime,
    auth_context: Option<&CodexAppsAuthContext>,
    accessible_connectors: &[AppInfo],
) -> anyhow::Result<Vec<DiscoverableTool>> {
    let connector_ids = tool_suggest_connector_ids(config, plugin_runtime).await;
    let directory_connectors = codex_connectors_api::merge::merge_plugin_connectors(
        cached_directory_connectors_for_tool_suggest_with_auth(config, auth_context).await,
        connector_ids.iter().cloned(),
    );
    let discoverable_connectors =
        codex_connectors_api::filter::filter_tool_suggest_discoverable_connectors(
            directory_connectors,
            accessible_connectors,
            &connector_ids,
            originator().value.as_str(),
        )
        .into_iter()
        .map(DiscoverableTool::from);
    let configured_plugin_ids = config
        .tool_suggest
        .discoverables
        .iter()
        .filter(|discoverable| discoverable.kind == ToolSuggestDiscoverableType::Plugin)
        .map(|discoverable| discoverable.id.clone())
        .collect::<HashSet<_>>();
    let disabled_plugin_ids = config
        .tool_suggest
        .disabled_tools
        .iter()
        .filter(|disabled_tool| disabled_tool.kind == ToolSuggestDiscoverableType::Plugin)
        .map(|disabled_tool| disabled_tool.id.clone())
        .collect::<HashSet<_>>();
    let discoverable_plugins = plugin_runtime
        .list_tool_suggest_discoverable_plugins(
            &config.plugins_config_input(),
            &configured_plugin_ids,
            &disabled_plugin_ids,
        )
        .await
        .map_err(anyhow::Error::msg)?
        .into_iter()
        .map(|plugin| {
            DiscoverableTool::from(DiscoverablePluginInfo {
                id: plugin.id,
                name: plugin.name,
                description: plugin.description,
                has_skills: plugin.has_skills,
                mcp_server_names: plugin.mcp_server_names,
                app_connector_ids: plugin.app_connector_ids,
            })
        });
    Ok(discoverable_connectors
        .chain(discoverable_plugins)
        .collect())
}

pub async fn list_accessible_and_enabled_connectors_from_manager(
    mcp_connection_manager: &dyn McpConnectionRuntime,
    config: &Config,
) -> Vec<AppInfo> {
    crate::with_app_enabled_state(
        accessible_connectors_from_mcp_tools(&mcp_connection_manager.list_all_tools().await),
        config,
    )
    .into_iter()
    .filter(|connector| connector.is_accessible && connector.is_enabled)
    .collect()
}

pub async fn list_cached_accessible_connectors_from_mcp_tools(
    config: &Config,
    auth_snapshot: Option<&RequestAuthSnapshot>,
) -> Option<Vec<AppInfo>> {
    let connector_auth_context = codex_apps_auth_context(auth_snapshot);
    if !config.features.apps_enabled_for_auth(
        connector_auth_context
            .as_ref()
            .is_some_and(|auth| auth.uses_codex_backend),
    ) {
        return Some(Vec::new());
    }
    let cache_key = accessible_connectors_cache_key(config, connector_auth_context.as_ref());
    read_cached_accessible_connectors(&cache_key).map(|connectors| {
        codex_connectors_api::filter::filter_disallowed_connectors(
            connectors,
            originator().value.as_str(),
        )
    })
}

pub fn refresh_accessible_connectors_cache_from_mcp_tools(
    config: &Config,
    auth_context: Option<&CodexAppsAuthContext>,
    mcp_tools: &[ToolInfo],
) {
    if !config.features.enabled(Feature::Apps) {
        return;
    }

    let cache_key = accessible_connectors_cache_key(config, auth_context);
    let accessible_connectors = codex_connectors_api::filter::filter_disallowed_connectors(
        accessible_connectors_from_mcp_tools(mcp_tools),
        originator().value.as_str(),
    );
    write_cached_accessible_connectors(cache_key, &accessible_connectors);
}

pub async fn list_accessible_connectors_from_mcp_tools_with_options(
    config: &Config,
    auth_snapshot: Option<&RequestAuthSnapshot>,
    force_refetch: bool,
    plugin_runtime: &dyn plugin_service_api::PluginRuntime,
    environment_provider: &dyn ExecEnvironmentProvider,
    mcp_auth_runtime: &dyn McpAuthRuntime,
    mcp_connection_runtime_factory: &dyn McpConnectionRuntimeFactory,
) -> anyhow::Result<Vec<AppInfo>> {
    Ok(
        list_accessible_connectors_from_mcp_tools_with_options_and_status(
            config,
            auth_snapshot,
            force_refetch,
            plugin_runtime,
            environment_provider,
            mcp_auth_runtime,
            mcp_connection_runtime_factory,
        )
        .await?
        .connectors,
    )
}

pub async fn list_accessible_connectors_from_mcp_tools_with_options_and_status(
    config: &Config,
    auth_snapshot: Option<&RequestAuthSnapshot>,
    force_refetch: bool,
    plugin_runtime: &dyn plugin_service_api::PluginRuntime,
    environment_provider: &dyn ExecEnvironmentProvider,
    mcp_auth_runtime: &dyn McpAuthRuntime,
    mcp_connection_runtime_factory: &dyn McpConnectionRuntimeFactory,
) -> anyhow::Result<AccessibleConnectorsStatus> {
    list_accessible_connectors_from_mcp_tools_with_environment_provider(
        config,
        auth_snapshot,
        force_refetch,
        plugin_runtime,
        environment_provider,
        mcp_auth_runtime,
        mcp_connection_runtime_factory,
    )
    .await
}

pub async fn list_accessible_connectors_from_mcp_tools_with_environment_provider(
    config: &Config,
    auth_snapshot: Option<&RequestAuthSnapshot>,
    force_refetch: bool,
    plugin_runtime: &dyn plugin_service_api::PluginRuntime,
    environment_provider: &dyn ExecEnvironmentProvider,
    mcp_auth_runtime: &dyn McpAuthRuntime,
    mcp_connection_runtime_factory: &dyn McpConnectionRuntimeFactory,
) -> anyhow::Result<AccessibleConnectorsStatus> {
    let connector_auth_context = codex_apps_auth_context(auth_snapshot);
    if !config.features.apps_enabled_for_auth(
        connector_auth_context
            .as_ref()
            .is_some_and(|auth| auth.uses_codex_backend),
    ) {
        return Ok(AccessibleConnectorsStatus {
            connectors: Vec::new(),
            codex_apps_ready: true,
        });
    }
    let cache_key = accessible_connectors_cache_key(config, connector_auth_context.as_ref());
    let mcp_config = config.to_mcp_config(plugin_runtime).await;
    let tool_plugin_provenance = tool_plugin_provenance(&mcp_config);
    if !force_refetch && let Some(cached_connectors) = read_cached_accessible_connectors(&cache_key)
    {
        let cached_connectors = codex_connectors_api::filter::filter_disallowed_connectors(
            cached_connectors,
            originator().value.as_str(),
        );
        let cached_connectors = with_app_plugin_sources(cached_connectors, &tool_plugin_provenance);
        return Ok(AccessibleConnectorsStatus {
            connectors: cached_connectors,
            codex_apps_ready: true,
        });
    }

    let auth_context = codex_apps_auth_context(auth_snapshot);
    let mcp_servers = with_codex_apps_mcp(HashMap::new(), auth_context.as_ref(), &mcp_config);
    let host_owned_codex_apps_enabled =
        host_owned_codex_apps_enabled(&mcp_config, auth_context.as_ref());
    if mcp_servers.is_empty() {
        return Ok(AccessibleConnectorsStatus {
            connectors: Vec::new(),
            codex_apps_ready: true,
        });
    }
    let codex_apps_startup_timeout = mcp_servers.get(CODEX_APPS_MCP_SERVER_NAME).map(|cfg| {
        cfg.configured_config()
            .and_then(|config| config.startup_timeout_sec)
            .unwrap_or(CONNECTORS_READY_TIMEOUT_ON_EMPTY_TOOLS)
    });

    let auth_status_entries = mcp_auth_runtime
        .compute_auth_statuses(
            mcp_servers
                .iter()
                .map(|(name, server)| (name.clone(), server.clone()))
                .collect(),
            config.mcp_oauth_credentials_store_mode,
            host_owned_codex_apps_enabled,
        )
        .await;

    let (tx_event, rx_event) = unbounded();
    drop(rx_event);

    let local_environment = environment_provider.local_environment();
    let environment = environment_provider
        .default_environment()
        .unwrap_or_else(|| Arc::clone(&local_environment));

    let mut mcp_runtime_start = mcp_connection_runtime_factory
        .start(McpConnectionRuntimeStartRequest {
            mcp_servers,
            store_mode: config.mcp_oauth_credentials_store_mode,
            auth_entries: auth_status_entries,
            approval_policy: config.permissions.approval_policy.clone(),
            submit_id: CONNECTORS_STARTUP_SUBMIT_ID.to_owned(),
            tx_event,
            initial_permission_profile: PermissionProfile::default(),
            runtime_environment: mcp_runtime_environment(
                environment,
                local_environment,
                config.cwd.to_path_buf(),
            ),
            codex_home: config.codex_home.to_path_buf(),
            codex_apps_tools_cache_key: codex_apps_tools_cache_key(auth_context.as_ref()),
            host_owned_codex_apps_enabled,
            client_elicitation_support: mcp_config.client_elicitation_support,
            tool_plugin_provenance: ToolPluginProvenance::default(),
            codex_apps_auth_provider: codex_apps_auth_provider(auth_snapshot),
            elicitation_reviewer: /*elicitation_reviewer*/ None,
        })
        .await;
    let mcp_connection_manager = mcp_runtime_start.runtime.as_mut();
    let cancel_token = mcp_runtime_start.startup_cancellation_token.clone();

    let refreshed_tools = if force_refetch {
        match mcp_connection_manager
            .hard_refresh_codex_apps_tools_cache()
            .await
        {
            Ok(tools) => Some(tools),
            Err(err) => {
                warn!(
                    "failed to force-refresh tools for MCP server '{CODEX_APPS_MCP_SERVER_NAME}', using cached/startup tools: {err:#}"
                );
                None
            }
        }
    } else {
        None
    };
    let refreshed_tools_succeeded = refreshed_tools.is_some();

    let mut tools = if let Some(tools) = refreshed_tools {
        tools
    } else {
        mcp_connection_manager.list_all_tools().await
    };
    let mut should_reload_tools = false;
    let codex_apps_ready = if refreshed_tools_succeeded {
        true
    } else if let Some(timeout) = codex_apps_startup_timeout {
        let immediate_ready = mcp_connection_manager
            .wait_for_server_ready(CODEX_APPS_MCP_SERVER_NAME, Duration::ZERO)
            .await;
        if immediate_ready {
            true
        } else if tools.is_empty() {
            let ready = mcp_connection_manager
                .wait_for_server_ready(CODEX_APPS_MCP_SERVER_NAME, timeout)
                .await;
            should_reload_tools = ready;
            ready
        } else {
            false
        }
    } else {
        false
    };
    if should_reload_tools {
        tools = mcp_connection_manager.list_all_tools().await;
    }
    if codex_apps_ready {
        cancel_token.cancel();
    }

    let accessible_connectors = codex_connectors_api::filter::filter_disallowed_connectors(
        accessible_connectors_from_mcp_tools(&tools),
        originator().value.as_str(),
    );
    if codex_apps_ready || !accessible_connectors.is_empty() {
        write_cached_accessible_connectors(cache_key, &accessible_connectors);
    }
    let accessible_connectors =
        with_app_plugin_sources(accessible_connectors, &tool_plugin_provenance);
    mcp_connection_manager.shutdown().await;
    Ok(AccessibleConnectorsStatus {
        connectors: accessible_connectors,
        codex_apps_ready,
    })
}

fn accessible_connectors_cache_key(
    config: &Config,
    auth: Option<&CodexAppsAuthContext>,
) -> AccessibleConnectorsCacheKey {
    let account_id = auth.and_then(|auth| auth.account_id.clone());
    let chatgpt_user_id = auth.and_then(|auth| auth.chatgpt_user_id.clone());
    let is_workspace_account = auth.is_some_and(|auth| auth.is_workspace_account);
    AccessibleConnectorsCacheKey {
        chatgpt_base_url: config.chatgpt_base_url.clone(),
        account_id,
        chatgpt_user_id,
        is_workspace_account,
    }
}

async fn tool_suggest_connector_ids(
    config: &Config,
    plugin_runtime: &dyn PluginRuntime,
) -> HashSet<String> {
    let plugins_input = config.plugins_config_input();
    let mut connector_ids = plugin_runtime
        .connector_ids_for_config(&plugins_input)
        .await;
    connector_ids.extend(
        config
            .tool_suggest
            .discoverables
            .iter()
            .filter(|discoverable| discoverable.kind == ToolSuggestDiscoverableType::Connector)
            .map(|discoverable| discoverable.id.clone()),
    );
    let disabled_connector_ids = config
        .tool_suggest
        .disabled_tools
        .iter()
        .filter(|disabled_tool| disabled_tool.kind == ToolSuggestDiscoverableType::Connector)
        .map(|disabled_tool| disabled_tool.id.as_str())
        .collect::<HashSet<_>>();
    connector_ids.retain(|connector_id| !disabled_connector_ids.contains(connector_id.as_str()));
    connector_ids
}

async fn cached_directory_connectors_for_tool_suggest_with_auth(
    config: &Config,
    auth_context: Option<&CodexAppsAuthContext>,
) -> Vec<AppInfo> {
    if !config.features.enabled(Feature::Apps) {
        return Vec::new();
    }

    let Some(auth_context) = auth_context.filter(|auth| auth.uses_codex_backend) else {
        return Vec::new();
    };

    let account_id = match auth_context.account_id.as_deref() {
        Some(account_id) if !account_id.is_empty() => account_id.to_string(),
        _ => return Vec::new(),
    };
    let cache_context = ConnectorDirectoryCacheContext::new(
        config.codex_home.to_path_buf(),
        ConnectorDirectoryCacheKey::new(
            config.chatgpt_base_url.clone(),
            Some(account_id),
            auth_context.chatgpt_user_id.clone(),
            auth_context.is_workspace_account,
        ),
    );

    codex_connectors_api::cached_directory_connectors(&cache_context).unwrap_or_default()
}

fn read_cached_accessible_connectors(
    cache_key: &AccessibleConnectorsCacheKey,
) -> Option<Vec<AppInfo>> {
    let mut cache_guard = ACCESSIBLE_CONNECTORS_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let now = Instant::now();

    if let Some(cached) = cache_guard.as_ref() {
        if now < cached.expires_at && cached.key == *cache_key {
            return Some(cached.connectors.clone());
        }
        if now >= cached.expires_at {
            *cache_guard = None;
        }
    }

    None
}

fn write_cached_accessible_connectors(
    cache_key: AccessibleConnectorsCacheKey,
    connectors: &[AppInfo],
) {
    let mut cache_guard = ACCESSIBLE_CONNECTORS_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *cache_guard = Some(CachedAccessibleConnectors {
        key: cache_key,
        expires_at: Instant::now() + codex_connectors_api::CONNECTORS_CACHE_TTL,
        connectors: connectors.to_vec(),
    });
}

pub fn accessible_connectors_from_mcp_tools(mcp_tools: &[ToolInfo]) -> Vec<AppInfo> {
    let tools = mcp_tools.iter().filter_map(|tool| {
        if tool.server_name != CODEX_APPS_MCP_SERVER_NAME {
            return None;
        }
        let connector_id = tool.connector_id.as_deref()?;
        Some(codex_connectors_api::accessible::AccessibleConnectorTool {
            connector_id: connector_id.to_string(),
            connector_name: tool.connector_name.clone(),
            connector_description: tool.namespace_description.clone(),
            plugin_display_names: tool.plugin_display_names.clone(),
        })
    });
    codex_connectors_api::accessible::collect_accessible_connectors(tools)
}

pub fn with_app_plugin_sources(
    mut connectors: Vec<AppInfo>,
    tool_plugin_provenance: &ToolPluginProvenance,
) -> Vec<AppInfo> {
    for connector in &mut connectors {
        connector.plugin_display_names = tool_plugin_provenance
            .plugin_display_names_for_connector_id(connector.id.as_str())
            .to_vec();
    }
    connectors
}

#[cfg(test)]
mod tests {
    use config_service::ConfigBuilder;
    use codex_connectors_api::metadata::connector_install_url;
    use codex_connectors_api::metadata::sanitize_name;
    use codex_features::Feature;
    use mcp_types::McpTool;
    use mcp_types::ToolAnnotations;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::*;

    fn test_tool_definition(tool_name: &str) -> McpTool {
        McpTool {
            name: tool_name.to_string(),
            title: None,
            description: None,
            input_schema: serde_json::Value::Object(serde_json::Map::new()),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        }
    }

    fn plugin_names(names: &[&str]) -> Vec<String> {
        names.iter().map(ToString::to_string).collect()
    }

    fn codex_app_tool(
        tool_name: &str,
        connector_id: &str,
        connector_name: Option<&str>,
        plugin_display_names: &[&str],
    ) -> ToolInfo {
        let tool_namespace = connector_name
            .map(sanitize_name)
            .map(|connector_name| format!("mcp__{CODEX_APPS_MCP_SERVER_NAME}__{connector_name}"))
            .unwrap_or_else(|| CODEX_APPS_MCP_SERVER_NAME.to_string());

        ToolInfo {
            server_name: CODEX_APPS_MCP_SERVER_NAME.to_string(),
            supports_parallel_tool_calls: false,
            server_origin: None,
            callable_name: tool_name.to_string(),
            callable_namespace: tool_namespace,
            namespace_description: None,
            tool: test_tool_definition(tool_name),
            connector_id: Some(connector_id.to_string()),
            connector_name: connector_name.map(ToOwned::to_owned),
            plugin_display_names: plugin_names(plugin_display_names),
        }
    }

    fn with_accessible_connectors_cache_cleared<R>(f: impl FnOnce() -> R) -> R {
        let previous = {
            let mut cache_guard = ACCESSIBLE_CONNECTORS_CACHE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            cache_guard.take()
        };
        let result = f();
        let mut cache_guard = ACCESSIBLE_CONNECTORS_CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *cache_guard = previous;
        result
    }

    #[test]
    fn accessible_connectors_from_mcp_tools_carries_plugin_display_names() {
        let tools = vec![
            codex_app_tool(
                "calendar_list_events",
                "calendar",
                /*connector_name*/ None,
                &["sample", "sample"],
            ),
            codex_app_tool(
                "calendar_create_event",
                "calendar",
                Some("Google Calendar"),
                &["beta", "sample"],
            ),
            ToolInfo {
                server_name: "sample".to_string(),
                supports_parallel_tool_calls: false,
                server_origin: None,
                callable_name: "echo".to_string(),
                callable_namespace: "sample".to_string(),
                namespace_description: None,
                tool: test_tool_definition("echo"),
                connector_id: None,
                connector_name: None,
                plugin_display_names: plugin_names(&["ignored"]),
            },
        ];

        let connectors = accessible_connectors_from_mcp_tools(&tools);

        assert_eq!(
            connectors,
            vec![AppInfo {
                id: "calendar".to_string(),
                name: "Google Calendar".to_string(),
                description: None,
                logo_url: None,
                logo_url_dark: None,
                distribution_channel: None,
                install_url: Some(connector_install_url("Google Calendar", "calendar")),
                branding: None,
                app_metadata: None,
                labels: None,
                is_accessible: true,
                is_enabled: true,
                plugin_display_names: plugin_names(&["beta", "sample"]),
            }]
        );
    }

    #[tokio::test]
    async fn refresh_accessible_connectors_cache_from_mcp_tools_writes_latest_installed_apps() {
        let codex_home = tempdir().expect("tempdir should succeed");
        let mut config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .build()
            .await
            .expect("config should load");
        let _ = config.features.set_enabled(Feature::Apps, /*enabled*/ true);
        let cache_key = accessible_connectors_cache_key(&config, /*auth*/ None);
        let tools = vec![
            codex_app_tool(
                "calendar_list_events",
                "calendar",
                Some("Google Calendar"),
                &["calendar-plugin"],
            ),
            codex_app_tool(
                "openai_hidden",
                "connector_openai_hidden",
                Some("Hidden"),
                &[],
            ),
        ];

        let cached = with_accessible_connectors_cache_cleared(|| {
            refresh_accessible_connectors_cache_from_mcp_tools(&config, /*auth*/ None, &tools);
            read_cached_accessible_connectors(&cache_key).expect("cache should be populated")
        });

        assert_eq!(
            cached,
            vec![
                AppInfo {
                    id: "calendar".to_string(),
                    name: "Google Calendar".to_string(),
                    description: None,
                    logo_url: None,
                    logo_url_dark: None,
                    distribution_channel: None,
                    install_url: Some(connector_install_url("Google Calendar", "calendar")),
                    branding: None,
                    app_metadata: None,
                    labels: None,
                    is_accessible: true,
                    is_enabled: true,
                    plugin_display_names: plugin_names(&["calendar-plugin"]),
                },
                AppInfo {
                    id: "connector_openai_hidden".to_string(),
                    name: "Hidden".to_string(),
                    description: None,
                    logo_url: None,
                    logo_url_dark: None,
                    distribution_channel: None,
                    install_url: Some(connector_install_url("Hidden", "connector_openai_hidden")),
                    branding: None,
                    app_metadata: None,
                    labels: None,
                    is_accessible: true,
                    is_enabled: true,
                    plugin_display_names: Vec::new(),
                }
            ]
        );
    }

    #[test]
    fn accessible_connectors_from_mcp_tools_preserves_description() {
        let mcp_tools = vec![ToolInfo {
            server_name: CODEX_APPS_MCP_SERVER_NAME.to_string(),
            supports_parallel_tool_calls: false,
            server_origin: None,
            callable_name: "calendar_create_event".to_string(),
            callable_namespace: "mcp__codex_apps__calendar".to_string(),
            namespace_description: Some("Plan events".to_string()),
            tool: McpTool {
                name: "calendar_create_event".to_string(),
                title: None,
                description: Some("Create a calendar event".to_string()),
                input_schema: serde_json::Value::Object(serde_json::Map::new()),
                output_schema: None,
                annotations: Some(ToolAnnotations {
                    destructive_hint: None,
                    idempotent_hint: None,
                    open_world_hint: None,
                    read_only_hint: None,
                    title: None,
                }),
                execution: None,
                icons: None,
                meta: None,
            },
            connector_id: Some("calendar".to_string()),
            connector_name: Some("Calendar".to_string()),
            plugin_display_names: Vec::new(),
        }];

        assert_eq!(
            accessible_connectors_from_mcp_tools(&mcp_tools),
            vec![AppInfo {
                id: "calendar".to_string(),
                name: "Calendar".to_string(),
                description: Some("Plan events".to_string()),
                logo_url: None,
                logo_url_dark: None,
                distribution_channel: None,
                branding: None,
                app_metadata: None,
                labels: None,
                install_url: Some(connector_install_url("Calendar", "calendar")),
                is_accessible: true,
                is_enabled: true,
                plugin_display_names: Vec::new(),
            }]
        );
    }
}
