use std::collections::HashMap;

use codex_config_types::McpServerConfig;
use protocol::protocol::McpAuthStatus;

#[derive(Debug, Clone)]
pub struct McpOAuthLoginConfig {
    pub url: String,
    pub http_headers: Option<HashMap<String, String>>,
    pub env_http_headers: Option<HashMap<String, String>>,
    pub discovered_scopes: Option<Vec<String>>,
}

#[derive(Debug)]
pub enum McpOAuthLoginSupport {
    Supported(McpOAuthLoginConfig),
    Unsupported,
    Unknown(anyhow::Error),
}

#[derive(Debug, Clone)]
pub struct McpAuthStatusEntry {
    pub config: Option<McpServerConfig>,
    pub auth_status: McpAuthStatus,
}
