use codex_config_types::McpServerConfig;

/// The launch strategy for an MCP server after runtime additions have been applied.
#[derive(Debug, Clone)]
enum McpServerLaunch {
    Configured(Box<McpServerConfig>),
}

/// MCP server after runtime additions have been applied.
#[derive(Debug, Clone)]
pub struct EffectiveMcpServer {
    launch: McpServerLaunch,
}

impl EffectiveMcpServer {
    pub fn configured(config: McpServerConfig) -> Self {
        Self {
            launch: McpServerLaunch::Configured(Box::new(config)),
        }
    }

    pub fn configured_config(&self) -> Option<&McpServerConfig> {
        match &self.launch {
            McpServerLaunch::Configured(config) => Some(config.as_ref()),
        }
    }

    pub fn enabled(&self) -> bool {
        match &self.launch {
            McpServerLaunch::Configured(config) => config.enabled,
        }
    }

    pub fn required(&self) -> bool {
        match &self.launch {
            McpServerLaunch::Configured(config) => config.required,
        }
    }
}
