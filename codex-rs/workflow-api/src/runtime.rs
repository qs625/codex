use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::WorkflowAbortArgs;
use crate::WorkflowDescribeArgs;
use crate::WorkflowDetails;
use crate::WorkflowDiscoveryContext;
use crate::WorkflowExecutionContext;
use crate::WorkflowRegistry;
use crate::WorkflowResumeArgs;
use crate::WorkflowRuntimeBridge;
use crate::WorkflowStartArgs;
use crate::WorkflowStatusArgs;
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

/// Narrow workflow service API consumed by tool handlers.
///
/// This is the global workflow domain service boundary. Handlers should depend
/// on this trait instead of assembling workflow controller, runtime bridge, and
/// progress side effects themselves.
pub trait WorkflowApi: Send + Sync + 'static {
    fn subscribe_workflow_updates(&self) -> Box<dyn WorkflowRunUpdateReceiver>;

    fn list_workflows<'a>(
        &'a self,
        discovery: WorkflowDiscoveryContext,
    ) -> Pin<Box<dyn Future<Output = Result<WorkflowRegistry, String>> + Send + 'a>>;

    fn describe_workflow<'a>(
        &'a self,
        discovery: WorkflowDiscoveryContext,
        args: WorkflowDescribeArgs,
    ) -> Pin<Box<dyn Future<Output = Result<WorkflowDetails, String>> + Send + 'a>>;

    fn start_workflow<'a>(
        &'a self,
        context: WorkflowExecutionContext,
        args: WorkflowStartArgs,
    ) -> WorkflowRunFuture<'a>;

    fn workflow_status<'a>(
        &'a self,
        args: WorkflowStatusArgs,
    ) -> WorkflowRunFuture<'a>;

    fn resume_workflow<'a>(
        &'a self,
        context: WorkflowExecutionContext,
        args: WorkflowResumeArgs,
    ) -> WorkflowRunFuture<'a>;

    fn abort_workflow<'a>(
        &'a self,
        context: WorkflowExecutionContext,
        args: WorkflowAbortArgs,
    ) -> WorkflowRunFuture<'a>;
}

impl<Service> WorkflowApi for Arc<Service>
where
    Service: WorkflowApi,
{
    fn subscribe_workflow_updates(&self) -> Box<dyn WorkflowRunUpdateReceiver> {
        self.as_ref().subscribe_workflow_updates()
    }

    fn list_workflows<'a>(
        &'a self,
        discovery: WorkflowDiscoveryContext,
    ) -> Pin<Box<dyn Future<Output = Result<WorkflowRegistry, String>> + Send + 'a>> {
        self.as_ref().list_workflows(discovery)
    }

    fn describe_workflow<'a>(
        &'a self,
        discovery: WorkflowDiscoveryContext,
        args: WorkflowDescribeArgs,
    ) -> Pin<Box<dyn Future<Output = Result<WorkflowDetails, String>> + Send + 'a>> {
        self.as_ref().describe_workflow(discovery, args)
    }

    fn start_workflow<'a>(
        &'a self,
        context: WorkflowExecutionContext,
        args: WorkflowStartArgs,
    ) -> WorkflowRunFuture<'a> {
        self.as_ref().start_workflow(context, args)
    }

    fn workflow_status<'a>(
        &'a self,
        args: WorkflowStatusArgs,
    ) -> WorkflowRunFuture<'a> {
        self.as_ref().workflow_status(args)
    }

    fn resume_workflow<'a>(
        &'a self,
        context: WorkflowExecutionContext,
        args: WorkflowResumeArgs,
    ) -> WorkflowRunFuture<'a> {
        self.as_ref().resume_workflow(context, args)
    }

    fn abort_workflow<'a>(
        &'a self,
        context: WorkflowExecutionContext,
        args: WorkflowAbortArgs,
    ) -> WorkflowRunFuture<'a> {
        self.as_ref().abort_workflow(context, args)
    }
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
