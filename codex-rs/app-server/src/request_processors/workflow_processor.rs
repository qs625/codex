use crate::config_manager::ConfigManager;
use crate::error_code::internal_error;
use crate::error_code::invalid_request;
use crate::outgoing_message::OutgoingMessageSender;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::WorkflowAbortParams;
use codex_app_server_protocol::WorkflowAbortResponse;
use codex_app_server_protocol::WorkflowDescribeParams;
use codex_app_server_protocol::WorkflowDescribeResponse;
use codex_app_server_protocol::WorkflowDetails as ApiWorkflowDetails;
use codex_app_server_protocol::WorkflowDiagnostic as ApiWorkflowDiagnostic;
use codex_app_server_protocol::WorkflowInputSpec as ApiWorkflowInputSpec;
use codex_app_server_protocol::WorkflowListParams;
use codex_app_server_protocol::WorkflowListResponse;
use codex_app_server_protocol::WorkflowResumeParams;
use codex_app_server_protocol::WorkflowResumeResponse;
use codex_app_server_protocol::WorkflowRun as ApiWorkflowRun;
use codex_app_server_protocol::WorkflowRunStatus as ApiWorkflowRunStatus;
use codex_app_server_protocol::WorkflowRunUpdatedNotification;
use codex_app_server_protocol::WorkflowSource as ApiWorkflowSource;
use codex_app_server_protocol::WorkflowStartParams;
use codex_app_server_protocol::WorkflowStartResponse;
use codex_app_server_protocol::WorkflowStatusParams;
use codex_app_server_protocol::WorkflowStatusResponse;
use codex_workflow::WorkflowRunManager;
use codex_workflow_api::WorkflowDetails;
use codex_workflow_api::WorkflowDiagnostic;
use codex_workflow_api::WorkflowInputSpec;
use codex_workflow_api::WorkflowRegistry;
use codex_workflow_api::WorkflowRun;
use codex_workflow_api::WorkflowRunStatus;
use codex_workflow_api::WorkflowSource;
use codex_workflow_api::WorkflowSummary;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct WorkflowRequestProcessor {
    config_manager: ConfigManager,
    outgoing: Arc<OutgoingMessageSender>,
    workflow_runs: Arc<WorkflowRunManager>,
}

impl WorkflowRequestProcessor {
    pub(crate) fn new(
        config_manager: ConfigManager,
        outgoing: Arc<OutgoingMessageSender>,
        codex_home: PathBuf,
    ) -> Self {
        Self {
            config_manager,
            outgoing,
            workflow_runs: Arc::new(WorkflowRunManager::new(codex_home)),
        }
    }

    pub(crate) async fn list(
        &self,
        params: WorkflowListParams,
    ) -> Result<WorkflowListResponse, JSONRPCErrorError> {
        let registry = self.registry(params.cwd).await?;
        Ok(WorkflowListResponse {
            workflows: registry
                .workflows
                .into_iter()
                .map(map_workflow_summary)
                .collect(),
            diagnostics: registry
                .diagnostics
                .into_iter()
                .map(map_workflow_diagnostic)
                .collect(),
        })
    }

    pub(crate) async fn describe(
        &self,
        params: WorkflowDescribeParams,
    ) -> Result<WorkflowDescribeResponse, JSONRPCErrorError> {
        let registry = self.registry(params.cwd).await?;
        let workflow = params.workflow.trim();
        if workflow.is_empty() {
            return Err(invalid_request("workflow must not be empty"));
        }
        let details = registry.details(workflow).map_err(invalid_request)?;
        Ok(WorkflowDescribeResponse {
            workflow: map_workflow_details(details),
        })
    }

    pub(crate) async fn start(
        &self,
        params: WorkflowStartParams,
    ) -> Result<WorkflowStartResponse, JSONRPCErrorError> {
        let registry = self.registry(params.cwd).await?;
        let workflow = params.workflow.trim();
        if workflow.is_empty() {
            return Err(invalid_request("workflow must not be empty"));
        }
        let updates = self.workflow_runs.subscribe();
        let run = self
            .workflow_runs
            .start(&registry, workflow, params.inputs)
            .await
            .map_err(invalid_request)?;
        self.send_run_updated(run.clone()).await;
        self.spawn_terminal_run_notification(run.run_id.clone(), updates);
        Ok(WorkflowStartResponse {
            run: map_workflow_run(run),
        })
    }

    pub(crate) async fn status(
        &self,
        params: WorkflowStatusParams,
    ) -> Result<WorkflowStatusResponse, JSONRPCErrorError> {
        let run_id = params.run_id.trim();
        if run_id.is_empty() {
            return Err(invalid_request("run_id must not be empty"));
        }
        let run = self
            .workflow_runs
            .status(run_id)
            .await
            .map_err(invalid_request)?;
        Ok(WorkflowStatusResponse {
            run: map_workflow_run(run),
        })
    }

    pub(crate) async fn resume(
        &self,
        params: WorkflowResumeParams,
    ) -> Result<WorkflowResumeResponse, JSONRPCErrorError> {
        let run_id = params.run_id.trim();
        if run_id.is_empty() {
            return Err(invalid_request("run_id must not be empty"));
        }
        let updates = self.workflow_runs.subscribe();
        let run = self
            .workflow_runs
            .resume(run_id, params.inputs)
            .await
            .map_err(invalid_request)?;
        self.send_run_updated(run.clone()).await;
        self.spawn_terminal_run_notification(run.run_id.clone(), updates);
        Ok(WorkflowResumeResponse {
            run: map_workflow_run(run),
        })
    }

    pub(crate) async fn abort(
        &self,
        params: WorkflowAbortParams,
    ) -> Result<WorkflowAbortResponse, JSONRPCErrorError> {
        let run_id = params.run_id.trim();
        if run_id.is_empty() {
            return Err(invalid_request("run_id must not be empty"));
        }
        let run = self
            .workflow_runs
            .abort(run_id, params.reason)
            .await
            .map_err(invalid_request)?;
        self.send_run_updated(run.clone()).await;
        Ok(WorkflowAbortResponse {
            run: map_workflow_run(run),
        })
    }

    async fn registry(&self, cwd: Option<String>) -> Result<WorkflowRegistry, JSONRPCErrorError> {
        let fallback_cwd = cwd.map(PathBuf::from);
        let config = self
            .config_manager
            .load_latest_config(fallback_cwd)
            .await
            .map_err(|err| internal_error(format!("failed to load workflow config: {err}")))?;
        Ok(codex_thread_runtime::workflows::load_workflow_registry(&config))
    }

    async fn send_run_updated(&self, run: WorkflowRun) {
        self.outgoing
            .send_server_notification(ServerNotification::WorkflowRunUpdated(
                WorkflowRunUpdatedNotification {
                    run: map_workflow_run(run),
                },
            ))
            .await;
    }

    fn spawn_terminal_run_notification(
        &self,
        run_id: String,
        mut updates: tokio::sync::broadcast::Receiver<WorkflowRun>,
    ) {
        let outgoing = Arc::clone(&self.outgoing);
        tokio::spawn(async move {
            loop {
                let run = match updates.recv().await {
                    Ok(run) => run,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                if run.run_id == run_id
                    && matches!(
                        run.status,
                        WorkflowRunStatus::Completed | WorkflowRunStatus::Failed
                    )
                {
                    outgoing
                        .send_server_notification(ServerNotification::WorkflowRunUpdated(
                            WorkflowRunUpdatedNotification {
                                run: map_workflow_run(run),
                            },
                        ))
                        .await;
                    break;
                }
            }
        });
    }
}

fn map_workflow_run(run: WorkflowRun) -> ApiWorkflowRun {
    ApiWorkflowRun {
        run_id: run.run_id,
        workflow: map_workflow_summary(run.workflow),
        status: map_workflow_run_status(run.status),
        runner_status: run.runner_status,
        inputs: run.inputs,
        created_at: run.created_at,
        updated_at: run.updated_at,
        revision: run.revision,
        message: run.message,
        abort_reason: run.abort_reason,
        output: run.output,
        error: run.error,
        snapshot_path: run.snapshot_path,
    }
}

fn map_workflow_run_status(status: WorkflowRunStatus) -> ApiWorkflowRunStatus {
    match status {
        WorkflowRunStatus::Running => ApiWorkflowRunStatus::Running,
        WorkflowRunStatus::Completed => ApiWorkflowRunStatus::Completed,
        WorkflowRunStatus::Failed => ApiWorkflowRunStatus::Failed,
        WorkflowRunStatus::Aborted => ApiWorkflowRunStatus::Aborted,
    }
}

fn map_workflow_details(details: WorkflowDetails) -> ApiWorkflowDetails {
    ApiWorkflowDetails {
        summary: map_workflow_summary(details.summary),
        instructions: details.instructions,
    }
}

fn map_workflow_summary(summary: WorkflowSummary) -> codex_app_server_protocol::WorkflowSummary {
    codex_app_server_protocol::WorkflowSummary {
        id: summary.id,
        name: summary.name,
        description: summary.description,
        source: map_workflow_source(summary.source),
        path: summary.path,
        entry: summary.entry,
        version: summary.version,
        when_to_use: summary.when_to_use,
        inputs: summary
            .inputs
            .into_iter()
            .map(|(key, value)| (key, map_workflow_input_spec(value)))
            .collect(),
    }
}

fn map_workflow_input_spec(spec: WorkflowInputSpec) -> ApiWorkflowInputSpec {
    ApiWorkflowInputSpec {
        input_type: spec.input_type,
        description: spec.description,
    }
}

fn map_workflow_diagnostic(diagnostic: WorkflowDiagnostic) -> ApiWorkflowDiagnostic {
    ApiWorkflowDiagnostic {
        source: map_workflow_source(diagnostic.source),
        path: diagnostic.path,
        message: diagnostic.message,
    }
}

fn map_workflow_source(source: WorkflowSource) -> ApiWorkflowSource {
    match source {
        WorkflowSource::Home => ApiWorkflowSource::Home,
        WorkflowSource::Project => ApiWorkflowSource::Project,
    }
}
