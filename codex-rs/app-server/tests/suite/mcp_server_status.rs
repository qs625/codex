use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use app_server_protocol::ListMcpServerStatusParams;
use app_server_protocol::ListMcpServerStatusResponse;
use app_server_protocol::McpServerStatusDetail;
use app_server_protocol::RequestId;
use app_test_support::McpProcess;
use app_test_support::create_mock_responses_server_sequence_unchecked;
use app_test_support::to_response;
use app_test_support::write_mock_responses_config_toml;
use axum::Router;
use pretty_assertions::assert_eq;
use rmcp::handler::server::ServerHandler;
use rmcp::model::JsonObject;
use rmcp::model::ListResourceTemplatesResult;
use rmcp::model::ListResourcesResult;
use rmcp::model::ListToolsResult;
use rmcp::model::PaginatedRequestParams;
use rmcp::model::ServerCapabilities;
use rmcp::model::ServerInfo;
use rmcp::model::Tool;
use rmcp::model::ToolAnnotations;
use rmcp::service::RequestContext;
use rmcp::transport::StreamableHttpServerConfig;
use rmcp::transport::StreamableHttpService;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use serde::de::DeserializeOwned;
use serde_json::json;
use std::path::Path;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(10);

fn write_mcp_server_status_config(
    codex_home: &Path,
    provider_uri: &str,
    mcp_servers: &[(&str, String)],
) -> Result<()> {
    write_mock_responses_config_toml(
        codex_home,
        provider_uri,
        &BTreeMap::new(),
        /*auto_compact_limit*/ 1024,
        /*requires_openai_auth*/ None,
        "mock_provider",
        "compact",
    )?;

    let config_path = codex_home.join("config.toml");
    let mut config_toml = std::fs::read_to_string(&config_path)?;
    for (name, url) in mcp_servers {
        config_toml.push_str(&format!(
            r#"
[mcp_servers.{name}]
url = "{url}/mcp"
"#
        ));
    }
    std::fs::write(config_path, config_toml)?;
    Ok(())
}

async fn init_mcp(codex_home: &Path) -> Result<McpProcess> {
    let mut mcp = McpProcess::new(codex_home).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;
    Ok(mcp)
}

async fn read_response<T: DeserializeOwned>(
    mcp: &mut McpProcess,
    request_id: i64,
    read_timeout: Duration,
) -> Result<T> {
    let response = timeout(
        read_timeout,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;
    to_response(response)
}

async fn list_status(
    mcp: &mut McpProcess,
    detail: Option<McpServerStatusDetail>,
    read_timeout: Duration,
) -> Result<ListMcpServerStatusResponse> {
    let request_id = mcp
        .send_list_mcp_server_status_request(ListMcpServerStatusParams {
            cursor: None,
            limit: None,
            detail,
        })
        .await?;
    read_response(mcp, request_id, read_timeout).await
}

fn lookup_tool(tool_name: String) -> Result<Tool, rmcp::ErrorData> {
    let input_schema: JsonObject = serde_json::from_value(json!({
        "type": "object",
        "additionalProperties": false
    }))
    .map_err(|err| rmcp::ErrorData::internal_error(err.to_string(), None))?;

    let mut tool = Tool::new(
        Cow::Owned(tool_name),
        Cow::Borrowed("Look up test data."),
        Arc::new(input_schema),
    );
    tool.annotations = Some(ToolAnnotations::new().read_only(true));
    Ok(tool)
}

#[tokio::test]
async fn mcp_server_status_list_returns_raw_server_and_tool_names() -> Result<()> {
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    let (mcp_server_url, mcp_server_handle) = start_mcp_server("look-up.raw", None).await?;
    let codex_home = TempDir::new()?;
    write_mcp_server_status_config(
        codex_home.path(),
        &server.uri(),
        &[("some-server", mcp_server_url)],
    )?;

    let mut mcp = init_mcp(codex_home.path()).await?;

    let response = list_status(&mut mcp, /*detail*/ None, DEFAULT_READ_TIMEOUT).await?;

    assert_eq!(response.next_cursor, None);
    assert_eq!(response.data.len(), 1);
    let status = &response.data[0];
    assert_eq!(status.name, "some-server");
    assert_eq!(
        status.tools.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from(["look-up.raw".to_string()])
    );
    assert_eq!(
        status
            .tools
            .get("look-up.raw")
            .map(|tool| tool.name.as_str()),
        Some("look-up.raw")
    );

    mcp_server_handle.abort();
    let _ = mcp_server_handle.await;

    Ok(())
}

#[derive(Clone)]
struct McpStatusServer {
    tool_name: Arc<String>,
    slow_inventory_delay: Option<Duration>,
}

impl ServerHandler for McpStatusServer {
    fn get_info(&self) -> ServerInfo {
        let capabilities = if self.slow_inventory_delay.is_some() {
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build()
        } else {
            ServerCapabilities::builder().enable_tools().build()
        };
        ServerInfo {
            capabilities,
            ..ServerInfo::default()
        }
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        Ok(ListToolsResult {
            tools: vec![lookup_tool(self.tool_name.as_ref().clone())?],
            next_cursor: None,
            meta: None,
        })
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ListResourcesResult, rmcp::ErrorData> {
        if let Some(delay) = self.slow_inventory_delay {
            tokio::time::sleep(delay).await;
        }
        Ok(ListResourcesResult {
            resources: Vec::new(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<rmcp::service::RoleServer>,
    ) -> Result<ListResourceTemplatesResult, rmcp::ErrorData> {
        if let Some(delay) = self.slow_inventory_delay {
            tokio::time::sleep(delay).await;
        }
        Ok(ListResourceTemplatesResult {
            resource_templates: Vec::new(),
            next_cursor: None,
            meta: None,
        })
    }
}

#[tokio::test]
async fn mcp_server_status_list_tools_and_auth_only_skips_slow_inventory_calls() -> Result<()> {
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    let (mcp_server_url, mcp_server_handle) =
        start_mcp_server("lookup", Some(Duration::from_secs(2))).await?;
    let codex_home = TempDir::new()?;
    write_mcp_server_status_config(
        codex_home.path(),
        &server.uri(),
        &[("some-server", mcp_server_url)],
    )?;

    let mut mcp = init_mcp(codex_home.path()).await?;

    let response = list_status(
        &mut mcp,
        Some(McpServerStatusDetail::ToolsAndAuthOnly),
        Duration::from_millis(500),
    )
    .await?;

    assert_eq!(response.next_cursor, None);
    assert_eq!(response.data.len(), 1);
    let status = &response.data[0];
    assert_eq!(status.name, "some-server");
    assert_eq!(
        status.tools.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from(["lookup".to_string()])
    );
    assert_eq!(status.resources, Vec::new());
    assert_eq!(status.resource_templates, Vec::new());

    mcp_server_handle.abort();
    let _ = mcp_server_handle.await;

    Ok(())
}

#[tokio::test]
async fn mcp_server_status_list_keeps_tools_for_sanitized_name_collisions() -> Result<()> {
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    let (dash_server_url, dash_server_handle) = start_mcp_server("dash_lookup", None).await?;
    let (underscore_server_url, underscore_server_handle) =
        start_mcp_server("underscore_lookup", None).await?;
    let codex_home = TempDir::new()?;
    write_mcp_server_status_config(
        codex_home.path(),
        &server.uri(),
        &[
            ("some-server", dash_server_url),
            ("some_server", underscore_server_url),
        ],
    )?;

    let mut mcp = init_mcp(codex_home.path()).await?;

    let response = list_status(&mut mcp, /*detail*/ None, DEFAULT_READ_TIMEOUT).await?;

    assert_eq!(response.next_cursor, None);
    assert_eq!(response.data.len(), 2);
    let status_tools = response
        .data
        .iter()
        .map(|status| {
            (
                status.name.as_str(),
                status.tools.keys().cloned().collect::<BTreeSet<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        status_tools,
        BTreeMap::from([
            ("some-server", BTreeSet::from(["dash_lookup".to_string()])),
            (
                "some_server",
                BTreeSet::from(["underscore_lookup".to_string()])
            )
        ])
    );

    dash_server_handle.abort();
    let _ = dash_server_handle.await;
    underscore_server_handle.abort();
    let _ = underscore_server_handle.await;

    Ok(())
}

async fn start_mcp_server(
    tool_name: &str,
    slow_inventory_delay: Option<Duration>,
) -> Result<(String, JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let tool_name = Arc::new(tool_name.to_string());
    let mcp_service = StreamableHttpService::new(
        move || {
            Ok(McpStatusServer {
                tool_name: Arc::clone(&tool_name),
                slow_inventory_delay,
            })
        },
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let router = Router::new().nest_service("/mcp", mcp_service);

    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    Ok((format!("http://{addr}"), handle))
}
