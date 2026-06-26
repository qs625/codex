use std::sync::Arc;

use codex_protocol::models::WorkflowRunProgressKind;
use codex_workflow_api::WorkflowAbortArgs;
use codex_workflow_api::WorkflowApi;
use codex_workflow_api::WorkflowCapability;
use codex_workflow_api::WorkflowProgressSink;
use codex_workflow_api::WorkflowRunStatus;
use codex_workflow_api::WorkflowRunUpdateError;
use codex_workflow_api::WorkflowRunUpdateReceiver;
use codex_workflow_api::WorkflowStartArgs;
use codex_workflow_api::WorkflowStatusArgs;
use codex_workflow_api::WorkflowResumeArgs;

#[derive(Default)]
pub struct WorkflowService;

impl WorkflowService {
    pub fn new() -> Self {
        Self
    }
}

impl WorkflowApi for WorkflowService {
    fn list_workflows<'a>(
        &'a self,
        capability: &'a dyn WorkflowCapability,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<codex_workflow_api::WorkflowRegistry, String>> + Send + 'a>> {
        Box::pin(async move { Ok(capability.load_workflow_registry()) })
    }

    fn describe_workflow<'a>(
        &'a self,
        capability: &'a dyn WorkflowCapability,
        args: codex_workflow_api::WorkflowDescribeArgs,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<codex_workflow_api::WorkflowDetails, String>> + Send + 'a>> {
        Box::pin(async move {
            let workflow = args.workflow().map(str::to_string)?;
            capability
                .load_workflow_registry()
                .details(workflow.as_str())
                .map_err(|err| err.to_string())
        })
    }

    fn start_workflow<'a>(
        &'a self,
        capability: &'a dyn WorkflowCapability,
        args: WorkflowStartArgs,
    ) -> codex_workflow_api::WorkflowRunFuture<'a> {
        Box::pin(async move {
            let workflow_id = args.workflow().map(str::to_string)?;
            let registry = capability.load_workflow_registry();
            let controller = capability.workflow_run_controller();
            let updates = controller.subscribe();
            let bridge = capability.create_workflow_runtime_bridge();
            let run = controller
                .start_with_bridge(
                    &registry,
                    &workflow_id,
                    args.inputs.unwrap_or_default(),
                    bridge,
                )
                .await?;
            let progress_sink = capability.workflow_progress_sink();
            progress_sink
                .record_workflow_progress(&run, WorkflowRunProgressKind::Started)
                .await;
            record_terminal_workflow_progress(progress_sink, updates, run.run_id.clone());
            Ok(run)
        })
    }

    fn workflow_status<'a>(
        &'a self,
        capability: &'a dyn WorkflowCapability,
        args: WorkflowStatusArgs,
    ) -> codex_workflow_api::WorkflowRunFuture<'a> {
        Box::pin(async move {
            let run_id = args.run_id()?;
            capability.workflow_run_controller().status(run_id).await
        })
    }

    fn resume_workflow<'a>(
        &'a self,
        capability: &'a dyn WorkflowCapability,
        args: WorkflowResumeArgs,
    ) -> codex_workflow_api::WorkflowRunFuture<'a> {
        Box::pin(async move {
            let run_id = args.run_id().map(str::to_string)?;
            let controller = capability.workflow_run_controller();
            let updates = controller.subscribe();
            let bridge = capability.create_workflow_runtime_bridge();
            let run = controller
                .resume_with_bridge(&run_id, args.inputs, bridge)
                .await?;
            let progress_sink = capability.workflow_progress_sink();
            progress_sink
                .record_workflow_progress(&run, WorkflowRunProgressKind::Resumed)
                .await;
            record_terminal_workflow_progress(progress_sink, updates, run.run_id.clone());
            Ok(run)
        })
    }

    fn abort_workflow<'a>(
        &'a self,
        capability: &'a dyn WorkflowCapability,
        args: WorkflowAbortArgs,
    ) -> codex_workflow_api::WorkflowRunFuture<'a> {
        Box::pin(async move {
            let run_id = args.run_id().map(str::to_string)?;
            let run = capability
                .workflow_run_controller()
                .abort(&run_id, args.reason)
                .await?;
            capability
                .workflow_progress_sink()
                .record_workflow_progress(&run, WorkflowRunProgressKind::Aborted)
                .await;
            Ok(run)
        })
    }
}

fn record_terminal_workflow_progress(
    progress_sink: Arc<dyn WorkflowProgressSink>,
    mut updates: Box<dyn WorkflowRunUpdateReceiver>,
    run_id: String,
) {
    tokio::spawn(async move {
        loop {
            let run = match updates.recv().await {
                Ok(run) => run,
                Err(WorkflowRunUpdateError::Lagged(_)) => continue,
                Err(WorkflowRunUpdateError::Closed) => break,
            };
            if run.run_id == run_id
                && let Some(kind) = workflow_progress_kind_for_terminal_status(run.status)
            {
                progress_sink.record_workflow_progress(&run, kind).await;
                break;
            }
        }
    });
}

fn workflow_progress_kind_for_terminal_status(
    status: WorkflowRunStatus,
) -> Option<WorkflowRunProgressKind> {
    match status {
        WorkflowRunStatus::Running => None,
        WorkflowRunStatus::Completed => Some(WorkflowRunProgressKind::Completed),
        WorkflowRunStatus::Failed => Some(WorkflowRunProgressKind::Failed),
        WorkflowRunStatus::Aborted => None,
    }
}
