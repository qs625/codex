use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::WorkflowRegistry;
use crate::WorkflowSummary;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAgentBinding {
    pub agent_id: String,
    pub agent_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Value>,
    #[serde(default)]
    pub options: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRuntimeRequest {
    pub run_id: String,
    pub workflow_id: String,
    pub rpc_id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRuntimeError {
    pub code: String,
    pub message: String,
}

impl WorkflowRuntimeError {
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self {
            code: "unsupported".to_string(),
            message: message.into(),
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_request".to_string(),
            message: message.into(),
        }
    }
}

/// Host implementation for workflow runtime requests.
///
/// Implementations must route requests through Codex runtime primitives so workflow scripts keep
/// the same permission, lifecycle, and typed event semantics as normal tool calls.
pub trait WorkflowRuntimeBridge: Send + Sync {
    fn call(
        &self,
        request: WorkflowRuntimeRequest,
    ) -> Pin<Box<dyn Future<Output = Result<Value, WorkflowRuntimeError>> + Send + '_>>;
}

pub type WorkflowRunResult = Result<WorkflowRun, String>;
pub type WorkflowRunFuture<'a> = Pin<Box<dyn Future<Output = WorkflowRunResult> + Send + 'a>>;
pub type WorkflowRunUpdateFuture<'a> =
    Pin<Box<dyn Future<Output = Result<WorkflowRun, WorkflowRunUpdateError>> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowRunUpdateError {
    Lagged(u64),
    Closed,
}

/// Receiver for workflow run progress updates.
///
/// Implementations adapt their internal notification mechanism into an object-safe async API so
/// runtime consumers do not depend on a specific channel implementation.
pub trait WorkflowRunUpdateReceiver: Send {
    fn recv(&mut self) -> WorkflowRunUpdateFuture<'_>;
}

/// Host-provided controller for workflow runs.
///
/// Implementations own persistence, runner process management, and update fanout. Consumers such
/// as `codex-core` should depend on this trait rather than the concrete workflow runtime crate.
pub trait WorkflowRunController: Send + Sync {
    fn subscribe(&self) -> Box<dyn WorkflowRunUpdateReceiver>;

    fn start_with_bridge<'a>(
        &'a self,
        registry: &'a WorkflowRegistry,
        workflow_id: &'a str,
        inputs: Value,
        bridge: Arc<dyn WorkflowRuntimeBridge>,
    ) -> WorkflowRunFuture<'a>;

    fn status<'a>(&'a self, run_id: &'a str) -> WorkflowRunFuture<'a>;

    fn resume_with_bridge<'a>(
        &'a self,
        run_id: &'a str,
        inputs: Option<Value>,
        bridge: Arc<dyn WorkflowRuntimeBridge>,
    ) -> WorkflowRunFuture<'a>;

    fn abort<'a>(&'a self, run_id: &'a str, reason: Option<String>) -> WorkflowRunFuture<'a>;
}

#[derive(Debug, Default)]
pub struct DisabledWorkflowRunController;

struct ClosedWorkflowRunUpdateReceiver;

impl WorkflowRunUpdateReceiver for ClosedWorkflowRunUpdateReceiver {
    fn recv(&mut self) -> WorkflowRunUpdateFuture<'_> {
        Box::pin(async { Err(WorkflowRunUpdateError::Closed) })
    }
}

impl DisabledWorkflowRunController {
    fn disabled_future() -> WorkflowRunFuture<'static> {
        Box::pin(async { Err("workflow runtime is not configured".to_string()) })
    }
}

impl WorkflowRunController for DisabledWorkflowRunController {
    fn subscribe(&self) -> Box<dyn WorkflowRunUpdateReceiver> {
        Box::new(ClosedWorkflowRunUpdateReceiver)
    }

    fn start_with_bridge<'a>(
        &'a self,
        _registry: &'a WorkflowRegistry,
        _workflow_id: &'a str,
        _inputs: Value,
        _bridge: Arc<dyn WorkflowRuntimeBridge>,
    ) -> WorkflowRunFuture<'a> {
        Self::disabled_future()
    }

    fn status<'a>(&'a self, _run_id: &'a str) -> WorkflowRunFuture<'a> {
        Self::disabled_future()
    }

    fn resume_with_bridge<'a>(
        &'a self,
        _run_id: &'a str,
        _inputs: Option<Value>,
        _bridge: Arc<dyn WorkflowRuntimeBridge>,
    ) -> WorkflowRunFuture<'a> {
        Self::disabled_future()
    }

    fn abort<'a>(&'a self, _run_id: &'a str, _reason: Option<String>) -> WorkflowRunFuture<'a> {
        Self::disabled_future()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunStatus {
    Running,
    Completed,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub run_id: String,
    pub workflow: WorkflowSummary,
    pub status: WorkflowRunStatus,
    pub runner_status: String,
    pub inputs: Value,
    pub created_at: i64,
    pub updated_at: i64,
    pub revision: u64,
    pub message: String,
    pub abort_reason: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bindings: BTreeMap<String, WorkflowAgentBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_path: Option<String>,
}
