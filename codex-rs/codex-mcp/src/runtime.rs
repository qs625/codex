//! Runtime support for Model Context Protocol (MCP) servers.
//!
//! This module contains data that describes the runtime environment in which MCP
//! servers execute, plus the sandbox state payload sent to capable servers and a
//! tiny shared metrics helper. Transport startup and orchestration live in
//! [`crate::rmcp_client`] and [`crate::connection_manager`].

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use codex_exec_server_api::ExecBackend;
use codex_exec_server_api::HttpClient;
pub use codex_mcp_types::SandboxState;

/// Runtime placement information used when starting MCP server transports.
///
/// `McpConfig` describes what servers exist. This value describes where those
/// servers should run for the current caller. Keep it explicit at manager
/// construction time so status/snapshot paths and real sessions make the same
/// local-vs-remote decision. `fallback_cwd` is not a per-server override; it is
/// used when a stdio server omits `cwd` and the launcher needs a concrete
/// process working directory.
#[derive(Clone)]
pub struct McpRuntimeEnvironment {
    remote_available: bool,
    remote_exec_backend: Arc<dyn ExecBackend>,
    local_http_client: Arc<dyn HttpClient>,
    remote_http_client: Arc<dyn HttpClient>,
    fallback_cwd: PathBuf,
}

pub struct McpRuntimeEnvironmentParams {
    pub remote_available: bool,
    pub remote_exec_backend: Arc<dyn ExecBackend>,
    pub local_http_client: Arc<dyn HttpClient>,
    pub remote_http_client: Arc<dyn HttpClient>,
    pub fallback_cwd: PathBuf,
}

impl McpRuntimeEnvironment {
    pub fn new(params: McpRuntimeEnvironmentParams) -> Self {
        let McpRuntimeEnvironmentParams {
            remote_available,
            remote_exec_backend,
            local_http_client,
            remote_http_client,
            fallback_cwd,
        } = params;
        Self {
            remote_available,
            remote_exec_backend,
            local_http_client,
            remote_http_client,
            fallback_cwd,
        }
    }

    pub(crate) fn remote_available(&self) -> bool {
        self.remote_available
    }

    pub(crate) fn remote_exec_backend(&self) -> Arc<dyn ExecBackend> {
        Arc::clone(&self.remote_exec_backend)
    }

    pub(crate) fn http_client(&self, remote: bool) -> Arc<dyn HttpClient> {
        if remote {
            Arc::clone(&self.remote_http_client)
        } else {
            Arc::clone(&self.local_http_client)
        }
    }

    pub(crate) fn fallback_cwd(&self) -> PathBuf {
        self.fallback_cwd.clone()
    }
}

pub(crate) fn emit_duration(metric: &str, duration: Duration, tags: &[(&str, &str)]) {
    codex_metrics_api::record_global_duration(metric, duration, tags);
}
