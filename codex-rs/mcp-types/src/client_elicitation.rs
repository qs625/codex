/// Client-side MCP elicitation support advertised by Codex.
///
/// This is a small semantic config value. The `codex-mcp` runtime owns
/// conversion to `rmcp::model::ElicitationCapability` at the protocol boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum McpClientElicitationSupport {
    #[default]
    Disabled,
    AuthElicitation,
}

impl McpClientElicitationSupport {
    pub fn from_auth_elicitation_enabled(enabled: bool) -> Self {
        if enabled {
            Self::AuthElicitation
        } else {
            Self::Disabled
        }
    }
}
