//! Read-only MCP and local-backend access to Codex memories.
//!
//! This module exposes tools for discovering and reading memory files. The
//! policy that tells a model when to use those tools is injected elsewhere.

pub mod backend;
pub mod local;

mod extension_tools;
mod schema;
mod server;

pub use backend::MemoriesBackend;
pub use backend::MemoriesBackendError;
pub use extension_tools::LIST_TOOL_NAME;
pub use extension_tools::MEMORY_TOOLS_NAMESPACE;
pub use extension_tools::READ_TOOL_NAME;
pub use extension_tools::SEARCH_TOOL_NAME;
pub use extension_tools::memory_extension_tool_name;
pub use extension_tools::memory_extension_tools;
pub use local::LocalMemoriesBackend;
pub use server::MemoriesMcpServer;
pub use server::run_server;
pub use server::run_stdio_server;
