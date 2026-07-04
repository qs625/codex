//! Runtime support for Model Context Protocol (MCP) servers.
//!
//! This module contains data that describes the runtime environment in which MCP
//! servers execute, plus the sandbox state payload sent to capable servers and a
//! tiny shared metrics helper. Transport startup and orchestration live in
//! [`crate::rmcp_client`] and [`crate::connection_manager`].

use std::time::Duration;

pub use mcp_types::SandboxState;
pub(crate) fn emit_duration(metric: &str, duration: Duration, tags: &[(&str, &str)]) {
    metrics_api::record_global_duration(metric, duration, tags);
}
