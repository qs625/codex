use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use std::time::Instant;

use async_channel::unbounded;
use codex_auth_types::RequestAuthSnapshot;
use codex_connectors_api::ConnectorDirectoryCacheContext;
use codex_connectors_api::ConnectorDirectoryCacheKey;
pub use codex_connectors_types::AppBranding;
pub use codex_connectors_types::AppInfo;
pub use codex_connectors_types::AppMetadata;
use codex_exec_server_api::ExecEnvironmentProvider;
use codex_protocol::models::PermissionProfile;
use codex_tool_planning::DiscoverableTool;
use serde::Deserialize;
use tracing::warn;

use crate::config::Config;
use crate::plugins::list_tool_suggest_discoverable_plugins;
use crate::session::INITIAL_SUBMIT_ID;
use codex_client_identity::originator;
use codex_config_requirements::AppsRequirementsToml;
use codex_config_types::AppToolApproval;
use codex_config_types::AppsConfigToml;
use codex_config_types::ToolSuggestDiscoverableType;
use codex_core_plugins_api::PluginRuntime;
use codex_features::Feature;
use codex_mcp_runtime_api::McpAuthRuntime;
use codex_mcp_runtime_api::McpConnectionRuntime;
use codex_mcp_runtime_api::McpConnectionRuntimeFactory;
use codex_mcp_runtime_api::McpConnectionRuntimeStartRequest;
use codex_mcp_tool_types::ToolAnnotations;
use codex_mcp_tool_types::ToolInfo;
use codex_mcp_types::CODEX_APPS_MCP_SERVER_NAME;
use codex_mcp_types::CodexAppsAuthContext;
use codex_mcp_types::ToolPluginProvenance;
use codex_mcp_types::codex_apps_tools_cache_key;
use codex_mcp_types::host_owned_codex_apps_enabled;
use codex_mcp_types::tool_plugin_provenance;
use codex_mcp_types::with_codex_apps_mcp;

const CONNECTORS_READY_TIMEOUT_ON_EMPTY_TOOLS: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AppToolPolicy {
    pub enabled: bool,
    pub approval: AppToolApproval,
}

impl Default for AppToolPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            approval: AppToolApproval::Auto,
        }
    }
}

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
    plugin_runtime: &dyn PluginRuntime,
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

pub(crate) async fn list_accessible_and_enabled_connectors_from_manager(
    mcp_connection_manager: &dyn McpConnectionRuntime,
    config: &Config,
) -> Vec<AppInfo> {
    with_app_enabled_state(
        accessible_connectors_from_mcp_tools(&mcp_connection_manager.list_all_tools().await),
        config,
    )
    .into_iter()
    .filter(|connector| connector.is_accessible && connector.is_enabled)
    .collect()
}

pub(crate) async fn list_tool_suggest_discoverable_tools_with_auth(
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
    let discoverable_plugins = list_tool_suggest_discoverable_plugins(config, plugin_runtime)
        .await?
        .into_iter()
        .map(DiscoverableTool::from);
    Ok(discoverable_connectors
        .chain(discoverable_plugins)
        .collect())
}

pub async fn list_cached_accessible_connectors_from_mcp_tools(
    config: &Config,
    auth_snapshot: Option<&RequestAuthSnapshot>,
) -> Option<Vec<AppInfo>> {
    let connector_auth_context = crate::mcp::codex_apps_auth_context(auth_snapshot);
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

pub(crate) fn refresh_accessible_connectors_cache_from_mcp_tools(
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
    plugin_runtime: &dyn PluginRuntime,
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
    plugin_runtime: &dyn PluginRuntime,
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
    plugin_runtime: &dyn PluginRuntime,
    environment_provider: &dyn ExecEnvironmentProvider,
    mcp_auth_runtime: &dyn McpAuthRuntime,
    mcp_connection_runtime_factory: &dyn McpConnectionRuntimeFactory,
) -> anyhow::Result<AccessibleConnectorsStatus> {
    let connector_auth_context = crate::mcp::codex_apps_auth_context(auth_snapshot);
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

    let auth_context = crate::mcp::codex_apps_auth_context(auth_snapshot);
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
            submit_id: INITIAL_SUBMIT_ID.to_owned(),
            tx_event,
            initial_permission_profile: PermissionProfile::default(),
            runtime_environment: crate::mcp::mcp_runtime_environment(
                environment,
                local_environment,
                config.cwd.to_path_buf(),
            ),
            codex_home: config.codex_home.to_path_buf(),
            codex_apps_tools_cache_key: codex_apps_tools_cache_key(auth_context.as_ref()),
            host_owned_codex_apps_enabled,
            client_elicitation_support: mcp_config.client_elicitation_support,
            tool_plugin_provenance: ToolPluginProvenance::default(),
            codex_apps_auth_provider: crate::mcp::codex_apps_auth_provider(auth_snapshot),
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

async fn tool_suggest_connector_ids(
    config: &Config,
    plugin_runtime: &dyn PluginRuntime,
) -> HashSet<String> {
    let plugins_input = config.plugins_config_input();
    let mut connector_ids = plugin_runtime
        .plugins_for_config(&plugins_input)
        .await
        .capability_summaries()
        .iter()
        .flat_map(|plugin| plugin.app_connector_ids.iter())
        .map(|connector_id| connector_id.0.clone())
        .collect::<HashSet<_>>();
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

pub(crate) fn accessible_connectors_from_mcp_tools(mcp_tools: &[ToolInfo]) -> Vec<AppInfo> {
    // ToolInfo already carries plugin provenance, so app-level plugin sources
    // can be derived here instead of requiring a separate enrichment pass.
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

pub fn with_app_enabled_state(mut connectors: Vec<AppInfo>, config: &Config) -> Vec<AppInfo> {
    let user_apps_config = read_user_apps_config(config);
    let requirements_apps_config = config.config_layer_stack.requirements_toml().apps.as_ref();
    if user_apps_config.is_none() && requirements_apps_config.is_none() {
        return connectors;
    }

    for connector in &mut connectors {
        if let Some(apps_config) = user_apps_config.as_ref()
            && (apps_config.default.is_some()
                || apps_config.apps.contains_key(connector.id.as_str()))
        {
            connector.is_enabled = app_is_enabled(apps_config, Some(connector.id.as_str()));
        }

        if requirements_apps_config
            .and_then(|apps| apps.apps.get(connector.id.as_str()))
            .is_some_and(|app| app.enabled == Some(false))
        {
            connector.is_enabled = false;
        }
    }

    connectors
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

pub(crate) fn app_tool_policy(
    config: &Config,
    connector_id: Option<&str>,
    tool_name: &str,
    tool_title: Option<&str>,
    annotations: Option<&ToolAnnotations>,
) -> AppToolPolicy {
    let apps_config = read_apps_config(config);
    let managed_approval = managed_app_tool_approval(
        config.config_layer_stack.requirements_toml().apps.as_ref(),
        connector_id,
        tool_name,
    );
    app_tool_policy_from_apps_config(
        apps_config.as_ref(),
        connector_id,
        tool_name,
        tool_title,
        annotations,
        managed_approval,
    )
}

pub(crate) fn codex_app_tool_is_enabled(config: &Config, tool_info: &ToolInfo) -> bool {
    if tool_info.server_name != CODEX_APPS_MCP_SERVER_NAME {
        return true;
    }

    app_tool_policy(
        config,
        tool_info.connector_id.as_deref(),
        &tool_info.tool.name,
        tool_info.tool.title.as_deref(),
        tool_info.tool.annotations.as_ref(),
    )
    .enabled
}

fn read_apps_config(config: &Config) -> Option<AppsConfigToml> {
    let apps_config = read_user_apps_config(config);
    let had_apps_config = apps_config.is_some();
    let mut apps_config = apps_config.unwrap_or_default();
    apply_requirements_apps_constraints(
        &mut apps_config,
        config.config_layer_stack.requirements_toml().apps.as_ref(),
    );
    if had_apps_config || apps_config.default.is_some() || !apps_config.apps.is_empty() {
        Some(apps_config)
    } else {
        None
    }
}

fn read_user_apps_config(config: &Config) -> Option<AppsConfigToml> {
    config
        .config_layer_stack
        .effective_config()
        .as_table()
        .and_then(|table| table.get("apps"))
        .cloned()
        .and_then(|value| AppsConfigToml::deserialize(value).ok())
}

fn apply_requirements_apps_constraints(
    apps_config: &mut AppsConfigToml,
    requirements_apps_config: Option<&AppsRequirementsToml>,
) {
    let Some(requirements_apps_config) = requirements_apps_config else {
        return;
    };

    for (app_id, requirement) in &requirements_apps_config.apps {
        if requirement.enabled == Some(false) {
            let app = apps_config.apps.entry(app_id.clone()).or_default();
            app.enabled = false;
        }
    }
}

fn managed_app_tool_approval(
    requirements_apps_config: Option<&AppsRequirementsToml>,
    connector_id: Option<&str>,
    tool_name: &str,
) -> Option<AppToolApproval> {
    let connector_id = connector_id?;
    requirements_apps_config?
        .apps
        .get(connector_id)?
        .tools
        .as_ref()?
        .tools
        .get(tool_name)?
        .approval_mode
}

fn app_is_enabled(apps_config: &AppsConfigToml, connector_id: Option<&str>) -> bool {
    let default_enabled = apps_config
        .default
        .as_ref()
        .map(|defaults| defaults.enabled)
        .unwrap_or(true);

    connector_id
        .and_then(|connector_id| apps_config.apps.get(connector_id))
        .map(|app| app.enabled)
        .unwrap_or(default_enabled)
}

fn app_tool_policy_from_apps_config(
    apps_config: Option<&AppsConfigToml>,
    connector_id: Option<&str>,
    tool_name: &str,
    tool_title: Option<&str>,
    annotations: Option<&ToolAnnotations>,
    managed_approval: Option<AppToolApproval>,
) -> AppToolPolicy {
    let Some(apps_config) = apps_config else {
        return AppToolPolicy {
            approval: managed_approval.unwrap_or(AppToolApproval::Auto),
            ..Default::default()
        };
    };

    let app = connector_id.and_then(|connector_id| apps_config.apps.get(connector_id));
    let tools = app.and_then(|app| app.tools.as_ref());
    let tool_config = tools.and_then(|tools| {
        tools
            .tools
            .get(tool_name)
            .or_else(|| tool_title.and_then(|title| tools.tools.get(title)))
    });
    let approval = managed_approval
        .or_else(|| tool_config.and_then(|tool| tool.approval_mode))
        .or_else(|| app.and_then(|app| app.default_tools_approval_mode))
        .unwrap_or(AppToolApproval::Auto);

    if !app_is_enabled(apps_config, connector_id) {
        return AppToolPolicy {
            enabled: false,
            approval,
        };
    }

    if let Some(enabled) = tool_config.and_then(|tool| tool.enabled) {
        return AppToolPolicy { enabled, approval };
    }

    if let Some(enabled) = app.and_then(|app| app.default_tools_enabled) {
        return AppToolPolicy { enabled, approval };
    }

    let app_defaults = apps_config.default.as_ref();
    let destructive_enabled = app
        .and_then(|app| app.destructive_enabled)
        .unwrap_or_else(|| {
            app_defaults
                .map(|defaults| defaults.destructive_enabled)
                .unwrap_or(true)
        });
    let open_world_enabled = app
        .and_then(|app| app.open_world_enabled)
        .unwrap_or_else(|| {
            app_defaults
                .map(|defaults| defaults.open_world_enabled)
                .unwrap_or(true)
        });
    let destructive_hint = annotations
        .and_then(|annotations| annotations.destructive_hint)
        .unwrap_or(true);
    let open_world_hint = annotations
        .and_then(|annotations| annotations.open_world_hint)
        .unwrap_or(true);
    let enabled =
        (destructive_enabled || !destructive_hint) && (open_world_enabled || !open_world_hint);

    AppToolPolicy { enabled, approval }
}

#[cfg(test)]
#[path = "connectors_tests.rs"]
mod tests;
