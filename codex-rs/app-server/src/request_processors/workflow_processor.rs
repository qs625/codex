use crate::config_manager::ConfigManager;
use crate::error_code::internal_error;
use crate::error_code::invalid_request;
use crate::outgoing_message::OutgoingMessageSender;
use app_server_protocol::JSONRPCErrorError;
use app_server_protocol::ServerNotification;
use app_server_protocol::WorkflowAbortParams;
use app_server_protocol::WorkflowAbortResponse;
use app_server_protocol::WorkflowDescribeParams;
use app_server_protocol::WorkflowDescribeResponse;
use app_server_protocol::WorkflowDetails as ApiWorkflowDetails;
use app_server_protocol::WorkflowDiagnostic as ApiWorkflowDiagnostic;
use app_server_protocol::WorkflowInputSpec as ApiWorkflowInputSpec;
use app_server_protocol::WorkflowListParams;
use app_server_protocol::WorkflowListResponse;
use app_server_protocol::WorkflowResumeParams;
use app_server_protocol::WorkflowResumeResponse;
use app_server_protocol::WorkflowRun as ApiWorkflowRun;
use app_server_protocol::WorkflowRunStatus as ApiWorkflowRunStatus;
use app_server_protocol::WorkflowRunUpdatedNotification;
use app_server_protocol::WorkflowSource as ApiWorkflowSource;
use app_server_protocol::WorkflowStartParams;
use app_server_protocol::WorkflowStartResponse;
use app_server_protocol::WorkflowStatusParams;
use app_server_protocol::WorkflowStatusResponse;
use codex_workflow_api::WorkflowApi;
use codex_workflow_api::WorkflowDetails;
use codex_workflow_api::WorkflowDiagnostic;
use codex_workflow_api::WorkflowDiscoveryContext;
use codex_workflow_api::WorkflowExecutionContext;
use codex_workflow_api::WorkflowInputSpec;
use codex_workflow_api::WorkflowRun;
use codex_workflow_api::WorkflowRunStatus;
use codex_workflow_api::WorkflowRunUpdateError;
use codex_workflow_api::WorkflowSource;
use codex_workflow_api::WorkflowSummary;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct WorkflowRequestProcessor {
    config_manager: ConfigManager,
    outgoing: Arc<OutgoingMessageSender>,
    workflow_api: Arc<dyn WorkflowApi>,
}

impl WorkflowRequestProcessor {
    pub(crate) fn new(
        config_manager: ConfigManager,
        outgoing: Arc<OutgoingMessageSender>,
        workflow_api: Arc<dyn WorkflowApi>,
    ) -> Self {
        Self {
            config_manager,
            outgoing,
            workflow_api,
        }
    }

    pub(crate) async fn list(
        &self,
        params: WorkflowListParams,
    ) -> Result<WorkflowListResponse, JSONRPCErrorError> {
        let discovery = self.discovery_context(params.cwd).await?;
        let workflows = self
            .workflow_api
            .list_workflows(discovery)
            .await
            .map_err(invalid_request)?;
        Ok(WorkflowListResponse {
            workflows: workflows
                .workflows
                .into_iter()
                .map(map_workflow_summary)
                .collect(),
            diagnostics: workflows
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
        let discovery = self.discovery_context(params.cwd).await?;
        let details = self
            .workflow_api
            .describe_workflow(
                discovery,
                codex_workflow_api::WorkflowDescribeArgs {
                    workflow: params.workflow,
                },
            )
            .await
            .map_err(invalid_request)?;
        Ok(WorkflowDescribeResponse {
            workflow: map_workflow_details(details),
        })
    }

    pub(crate) async fn start(
        &self,
        params: WorkflowStartParams,
    ) -> Result<WorkflowStartResponse, JSONRPCErrorError> {
        let discovery = self.discovery_context(params.cwd).await?;
        let updates = self.workflow_api.subscribe_workflow_updates();
        let run = self
            .workflow_api
            .start_workflow(
                WorkflowExecutionContext::new(discovery, None),
                codex_workflow_api::WorkflowStartArgs {
                    workflow: params.workflow,
                    inputs: Some(params.inputs),
                },
            )
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
        let run = self
            .workflow_api
            .workflow_status(codex_workflow_api::WorkflowStatusArgs {
                run_id: params.run_id,
            })
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
        let updates = self.workflow_api.subscribe_workflow_updates();
        let run = self
            .workflow_api
            .resume_workflow(
                WorkflowExecutionContext::new(empty_discovery_context(), None),
                codex_workflow_api::WorkflowResumeArgs {
                    run_id: params.run_id,
                    inputs: params.inputs,
                },
            )
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
        let run = self
            .workflow_api
            .abort_workflow(
                WorkflowExecutionContext::new(empty_discovery_context(), None),
                codex_workflow_api::WorkflowAbortArgs {
                    run_id: params.run_id,
                    reason: params.reason,
                },
            )
            .await
            .map_err(invalid_request)?;
        self.send_run_updated(run.clone()).await;
        Ok(WorkflowAbortResponse {
            run: map_workflow_run(run),
        })
    }

    async fn discovery_context(
        &self,
        cwd: Option<String>,
    ) -> Result<WorkflowDiscoveryContext, JSONRPCErrorError> {
        let fallback_cwd = cwd.map(PathBuf::from);
        let config = self
            .config_manager
            .load_latest_config(fallback_cwd)
            .await
            .map_err(|err| internal_error(format!("failed to load workflow config: {err}")))?;
        Ok(
            codex_workflow_api::workflow_discovery_context_from_config_layers(
                config.codex_home.as_ref(),
                config.cwd.as_ref(),
                config
                    .config_layer_stack
                    .get_layers(
                        config_service::ConfigLayerStackOrdering::LowestPrecedenceFirst,
                        /*include_disabled*/ false,
                    )
                    .into_iter()
                    .cloned()
                    .collect(),
            ),
        )
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
        mut updates: Box<dyn codex_workflow_api::WorkflowRunUpdateReceiver>,
    ) {
        let outgoing = Arc::clone(&self.outgoing);
        tokio::spawn(async move {
            loop {
                let run = match updates.recv().await {
                    Ok(run) => run,
                    Err(WorkflowRunUpdateError::Lagged(_)) => continue,
                    Err(WorkflowRunUpdateError::Closed) => break,
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

fn empty_discovery_context() -> WorkflowDiscoveryContext {
    WorkflowDiscoveryContext {
        home_root: PathBuf::new(),
        project_roots: Vec::new(),
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

fn map_workflow_summary(summary: WorkflowSummary) -> app_server_protocol::WorkflowSummary {
    app_server_protocol::WorkflowSummary {
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
