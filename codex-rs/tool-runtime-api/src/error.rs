use codex_protocol::error::CodexErr;

#[derive(Debug)]
pub enum ToolError {
    Rejected(String),
    Codex(CodexErr),
}
