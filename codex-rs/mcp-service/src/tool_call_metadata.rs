use codex_auth_types::RequestAuthSnapshot;
use codex_connectors_api::AppInfo;
use mcp_service_api::McpAppUsageMetadata;
use mcp_types::CODEX_APPS_MCP_SERVER_NAME;
use mcp_types::MCP_TOOL_CODEX_APPS_META_KEY;
use mcp_types::McpToolApprovalMetadata;
use mcp_types::ToolInfo;
use mcp_types::mcp_app_resource_uri_from_tool_meta;
use mcp_types::openai_file_input_params_for_server;

/// Host capabilities needed to look up MCP tool metadata.
///
/// Implementations own the concrete MCP manager, auth runtime, connector cache,
/// and network-backed connector fetch. The MCP runtime owns the decision flow
/// for when those capabilities are used and how they become approval metadata.
pub trait McpToolMetadataLookupHost {
    fn list_all_mcp_tools(&self) -> impl std::future::Future<Output = Vec<ToolInfo>> + Send;

    fn codex_apps_auth_snapshot(
        &self,
    ) -> impl std::future::Future<Output = Option<RequestAuthSnapshot>> + Send;

    fn cached_accessible_connectors<'a>(
        &'a self,
        auth_snapshot: Option<&'a RequestAuthSnapshot>,
    ) -> impl std::future::Future<Output = Option<Vec<AppInfo>>> + Send + 'a;

    fn fetch_accessible_connectors<'a>(
        &'a self,
        auth_snapshot: Option<&'a RequestAuthSnapshot>,
    ) -> impl std::future::Future<Output = anyhow::Result<Vec<AppInfo>>> + Send + 'a;
}

pub fn find_mcp_tool_info<'a>(
    tools: &'a [ToolInfo],
    server: &str,
    tool_name: &str,
) -> Option<&'a ToolInfo> {
    tools
        .iter()
        .find(|tool_info| tool_info.server_name == server && tool_info.tool.name == tool_name)
}

pub fn connector_description_for_tool(
    connectors: &[AppInfo],
    connector_id: Option<&str>,
) -> Option<String> {
    let connector_id = connector_id?;
    connectors
        .iter()
        .find(|connector| connector.id == connector_id)
        .and_then(|connector| connector.description.clone())
}

pub fn build_mcp_tool_approval_metadata(
    server: &str,
    tool_info: &ToolInfo,
    connector_description: Option<String>,
) -> McpToolApprovalMetadata {
    McpToolApprovalMetadata {
        annotations: tool_info.tool.annotations.clone(),
        connector_id: tool_info.connector_id.clone(),
        connector_name: tool_info.connector_name.clone(),
        connector_description,
        tool_title: tool_info.tool.title.clone(),
        tool_description: tool_info.tool.description.clone(),
        mcp_app_resource_uri: mcp_app_resource_uri_from_tool_meta(tool_info.tool.meta.as_ref()),
        codex_apps_meta: tool_info
            .tool
            .meta
            .as_ref()
            .and_then(|meta| meta.get(MCP_TOOL_CODEX_APPS_META_KEY))
            .and_then(serde_json::Value::as_object)
            .cloned(),
        // Disallow custom MCPs from uploading files via fileParams.
        openai_file_input_params: openai_file_input_params_for_server(
            server,
            tool_info.tool.meta.as_ref(),
        ),
    }
}

pub async fn lookup_mcp_tool_metadata(
    host: &impl McpToolMetadataLookupHost,
    server: &str,
    tool_name: &str,
) -> Option<McpToolApprovalMetadata> {
    let tools = host.list_all_mcp_tools().await;
    let tool_info = find_mcp_tool_info(&tools, server, tool_name)?.clone();
    let connector_description = if server == CODEX_APPS_MCP_SERVER_NAME {
        let auth_snapshot = host.codex_apps_auth_snapshot().await;
        let connectors = match host
            .cached_accessible_connectors(auth_snapshot.as_ref())
            .await
        {
            Some(connectors) => Some(connectors),
            None => host
                .fetch_accessible_connectors(auth_snapshot.as_ref())
                .await
                .ok(),
        };
        connectors.as_ref().and_then(|connectors| {
            connector_description_for_tool(connectors, tool_info.connector_id.as_deref())
        })
    } else {
        None
    };

    Some(build_mcp_tool_approval_metadata(
        server,
        &tool_info,
        connector_description,
    ))
}

pub fn lookup_mcp_app_usage_metadata(
    tools: &[ToolInfo],
    server: &str,
    tool_name: &str,
) -> Option<McpAppUsageMetadata> {
    find_mcp_tool_info(tools, server, tool_name).map(|tool_info| McpAppUsageMetadata {
        connector_id: tool_info.connector_id.clone(),
        app_name: tool_info.connector_name.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_auth_types::AuthMode;
    use codex_auth_types::BearerRequestAuthSnapshot;
    use codex_auth_types::RequestAuthSnapshot;
    use mcp_types::McpTool;
    use mcp_types::ToolAnnotations;
    use pretty_assertions::assert_eq;
    use std::sync::Mutex;

    fn tool_info(
        server: &str,
        tool_name: &str,
        connector_id: Option<&str>,
        connector_name: Option<&str>,
    ) -> ToolInfo {
        ToolInfo {
            server_name: server.to_string(),
            supports_parallel_tool_calls: false,
            server_origin: None,
            callable_name: tool_name.to_string(),
            callable_namespace: format!("mcp__{server}"),
            namespace_description: None,
            tool: McpTool {
                name: tool_name.to_string(),
                title: Some("Create Event".to_string()),
                description: Some("Create a calendar event".to_string()),
                input_schema: serde_json::json!({"type": "object"}),
                output_schema: None,
                annotations: Some(ToolAnnotations {
                    destructive_hint: Some(false),
                    idempotent_hint: None,
                    open_world_hint: Some(true),
                    read_only_hint: Some(false),
                    title: None,
                }),
                execution: None,
                icons: None,
                meta: Some(
                    serde_json::json!({
                        MCP_TOOL_CODEX_APPS_META_KEY: {
                            "resource_uri": "connector://calendar/tools/create_event",
                            "connector_id": "calendar",
                        },
                        "openai/outputTemplate": "connector://calendar/tools/create_event",
                        "openai/fileParams": ["attachment"],
                    })
                    .as_object()
                    .cloned()
                    .expect("meta object"),
                ),
            },
            connector_id: connector_id.map(str::to_string),
            connector_name: connector_name.map(str::to_string),
            plugin_display_names: Vec::new(),
        }
    }

    fn app_info(id: &str, description: Option<&str>) -> AppInfo {
        AppInfo {
            id: id.to_string(),
            name: id.to_string(),
            description: description.map(str::to_string),
            logo_url: None,
            logo_url_dark: None,
            distribution_channel: None,
            branding: None,
            app_metadata: None,
            labels: None,
            install_url: None,
            is_accessible: true,
            is_enabled: true,
            plugin_display_names: Vec::new(),
        }
    }

    struct FakeMetadataLookupHost {
        tools: Vec<ToolInfo>,
        auth_snapshot: Option<RequestAuthSnapshot>,
        cached_connectors: Option<Vec<AppInfo>>,
        fetched_connectors: Vec<AppInfo>,
        auth_count: Mutex<usize>,
        cache_count: Mutex<usize>,
        fetch_count: Mutex<usize>,
    }

    impl FakeMetadataLookupHost {
        fn new(tools: Vec<ToolInfo>) -> Self {
            Self {
                tools,
                auth_snapshot: Some(RequestAuthSnapshot::Bearer(BearerRequestAuthSnapshot {
                    auth_mode: AuthMode::Chatgpt,
                    token: None,
                    account_id: Some("account".to_string()),
                    chatgpt_user_id: Some("user".to_string()),
                    is_workspace_account: false,
                    is_fedramp_account: false,
                })),
                cached_connectors: None,
                fetched_connectors: Vec::new(),
                auth_count: Mutex::new(0),
                cache_count: Mutex::new(0),
                fetch_count: Mutex::new(0),
            }
        }

        fn with_cached_connectors(mut self, connectors: Vec<AppInfo>) -> Self {
            self.cached_connectors = Some(connectors);
            self
        }

        fn with_fetched_connectors(mut self, connectors: Vec<AppInfo>) -> Self {
            self.fetched_connectors = connectors;
            self
        }

        fn counts(&self) -> (usize, usize, usize) {
            (
                *self.auth_count.lock().expect("auth count lock"),
                *self.cache_count.lock().expect("cache count lock"),
                *self.fetch_count.lock().expect("fetch count lock"),
            )
        }
    }

    impl McpToolMetadataLookupHost for FakeMetadataLookupHost {
        async fn list_all_mcp_tools(&self) -> Vec<ToolInfo> {
            self.tools.clone()
        }

        async fn codex_apps_auth_snapshot(&self) -> Option<RequestAuthSnapshot> {
            *self.auth_count.lock().expect("auth count lock") += 1;
            self.auth_snapshot.clone()
        }

        async fn cached_accessible_connectors(
            &self,
            _auth_snapshot: Option<&RequestAuthSnapshot>,
        ) -> Option<Vec<AppInfo>> {
            *self.cache_count.lock().expect("cache count lock") += 1;
            self.cached_connectors.clone()
        }

        async fn fetch_accessible_connectors(
            &self,
            _auth_snapshot: Option<&RequestAuthSnapshot>,
        ) -> anyhow::Result<Vec<AppInfo>> {
            *self.fetch_count.lock().expect("fetch count lock") += 1;
            Ok(self.fetched_connectors.clone())
        }
    }

    #[test]
    fn builds_approval_metadata_from_tool_info() {
        let tool = tool_info(
            CODEX_APPS_MCP_SERVER_NAME,
            "create_event",
            Some("calendar"),
            Some("Calendar"),
        );

        let metadata = build_mcp_tool_approval_metadata(
            CODEX_APPS_MCP_SERVER_NAME,
            &tool,
            Some("Calendar connector".to_string()),
        );

        assert_eq!(metadata.connector_id.as_deref(), Some("calendar"));
        assert_eq!(metadata.connector_name.as_deref(), Some("Calendar"));
        assert_eq!(
            metadata.connector_description.as_deref(),
            Some("Calendar connector")
        );
        assert_eq!(metadata.tool_title.as_deref(), Some("Create Event"));
        assert_eq!(
            metadata.mcp_app_resource_uri.as_deref(),
            Some("connector://calendar/tools/create_event")
        );
        assert_eq!(
            metadata.openai_file_input_params,
            Some(vec!["attachment".to_string()])
        );
        assert_eq!(
            metadata
                .codex_apps_meta
                .as_ref()
                .and_then(|meta| meta.get("connector_id"))
                .and_then(serde_json::Value::as_str),
            Some("calendar")
        );
    }

    #[test]
    fn custom_server_does_not_accept_openai_file_params() {
        let tool = tool_info(
            "custom_server",
            "create_event",
            /*connector_id*/ None,
            /*connector_name*/ None,
        );

        let metadata = build_mcp_tool_approval_metadata(
            "custom_server",
            &tool,
            /*connector_description*/ None,
        );

        assert_eq!(metadata.openai_file_input_params, None);
    }

    #[test]
    fn connector_description_matches_by_connector_id() {
        let connectors = vec![
            app_info("mail", Some("Mail connector")),
            app_info("calendar", Some("Calendar connector")),
        ];

        assert_eq!(
            connector_description_for_tool(&connectors, Some("calendar")).as_deref(),
            Some("Calendar connector")
        );
        assert_eq!(
            connector_description_for_tool(&connectors, Some("missing")),
            None
        );
        assert_eq!(
            connector_description_for_tool(&connectors, /*connector_id*/ None),
            None
        );
    }

    #[test]
    fn app_usage_metadata_uses_matching_tool_connector() {
        let tools = vec![
            tool_info(
                "custom", "search", /*connector_id*/ None, /*connector_name*/ None,
            ),
            tool_info(
                CODEX_APPS_MCP_SERVER_NAME,
                "create_event",
                Some("calendar"),
                Some("Calendar"),
            ),
        ];

        assert_eq!(
            lookup_mcp_app_usage_metadata(&tools, CODEX_APPS_MCP_SERVER_NAME, "create_event"),
            Some(McpAppUsageMetadata {
                connector_id: Some("calendar".to_string()),
                app_name: Some("Calendar".to_string()),
            })
        );
        assert_eq!(
            lookup_mcp_app_usage_metadata(&tools, CODEX_APPS_MCP_SERVER_NAME, "missing"),
            None
        );
    }

    #[tokio::test]
    async fn metadata_lookup_uses_cached_connectors_for_codex_apps() {
        let host = FakeMetadataLookupHost::new(vec![tool_info(
            CODEX_APPS_MCP_SERVER_NAME,
            "create_event",
            Some("calendar"),
            Some("Calendar"),
        )])
        .with_cached_connectors(vec![app_info("calendar", Some("Cached Calendar"))]);

        let metadata = lookup_mcp_tool_metadata(&host, CODEX_APPS_MCP_SERVER_NAME, "create_event")
            .await
            .expect("metadata");

        assert_eq!(
            metadata.connector_description.as_deref(),
            Some("Cached Calendar")
        );
        assert_eq!(host.counts(), (1, 1, 0));
    }

    #[tokio::test]
    async fn metadata_lookup_fetches_connectors_when_cache_misses() {
        let host = FakeMetadataLookupHost::new(vec![tool_info(
            CODEX_APPS_MCP_SERVER_NAME,
            "create_event",
            Some("calendar"),
            Some("Calendar"),
        )])
        .with_fetched_connectors(vec![app_info("calendar", Some("Fetched Calendar"))]);

        let metadata = lookup_mcp_tool_metadata(&host, CODEX_APPS_MCP_SERVER_NAME, "create_event")
            .await
            .expect("metadata");

        assert_eq!(
            metadata.connector_description.as_deref(),
            Some("Fetched Calendar")
        );
        assert_eq!(host.counts(), (1, 1, 1));
    }

    #[tokio::test]
    async fn metadata_lookup_for_custom_server_does_not_query_connectors() {
        let host = FakeMetadataLookupHost::new(vec![tool_info(
            "custom_server",
            "search",
            /*connector_id*/ None,
            /*connector_name*/ None,
        )])
        .with_fetched_connectors(vec![app_info("calendar", Some("Fetched Calendar"))]);

        let metadata = lookup_mcp_tool_metadata(&host, "custom_server", "search")
            .await
            .expect("metadata");

        assert_eq!(metadata.connector_description, None);
        assert_eq!(host.counts(), (0, 0, 0));
    }
}
