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
    workflow_api: Arc<dyn WorkflowApi>,
    run_notifications: WorkflowRunNotifications,
}

impl WorkflowRequestProcessor {
    pub(crate) fn new(
        config_manager: ConfigManager,
        outgoing: Arc<OutgoingMessageSender>,
        workflow_api: Arc<dyn WorkflowApi>,
    ) -> Self {
        Self {
            config_manager,
            workflow_api,
            run_notifications: WorkflowRunNotifications::new(outgoing),
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
        Ok(WorkflowStartResponse {
            run: self
                .run_notifications
                .finish_started_run(run, updates)
                .await,
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
        Ok(WorkflowResumeResponse {
            run: self
                .run_notifications
                .finish_started_run(run, updates)
                .await,
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
        Ok(WorkflowAbortResponse {
            run: self.run_notifications.finish_run_update(run).await,
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
}

#[derive(Clone)]
struct WorkflowRunNotifications {
    outgoing: Arc<OutgoingMessageSender>,
}

impl WorkflowRunNotifications {
    fn new(outgoing: Arc<OutgoingMessageSender>) -> Self {
        Self { outgoing }
    }

    async fn finish_started_run(
        &self,
        run: WorkflowRun,
        updates: Box<dyn codex_workflow_api::WorkflowRunUpdateReceiver>,
    ) -> ApiWorkflowRun {
        let response_run = run.clone();
        self.send_run_updated(run.clone()).await;
        self.spawn_terminal_run_notification(run.run_id, updates);
        map_workflow_run(response_run)
    }

    async fn finish_run_update(&self, run: WorkflowRun) -> ApiWorkflowRun {
        let response_run = run.clone();
        self.send_run_updated(run).await;
        map_workflow_run(response_run)
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
                if run.run_id == run_id && is_terminal_workflow_run_notification_status(run.status)
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

fn is_terminal_workflow_run_notification_status(status: WorkflowRunStatus) -> bool {
    matches!(
        status,
        WorkflowRunStatus::Completed | WorkflowRunStatus::Failed
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outgoing_message::OutgoingEnvelope;
    use crate::outgoing_message::OutgoingMessage;
    use crate::outgoing_message::OutgoingMessageSender;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::collections::VecDeque;
    use std::future::Future;
    use std::pin::Pin;
    use tokio::sync::mpsc;

    struct TestWorkflowRunUpdates {
        runs: VecDeque<Result<WorkflowRun, WorkflowRunUpdateError>>,
    }

    impl TestWorkflowRunUpdates {
        fn new(runs: Vec<Result<WorkflowRun, WorkflowRunUpdateError>>) -> Self {
            Self { runs: runs.into() }
        }
    }

    impl codex_workflow_api::WorkflowRunUpdateReceiver for TestWorkflowRunUpdates {
        fn recv(
            &mut self,
        ) -> Pin<Box<dyn Future<Output = Result<WorkflowRun, WorkflowRunUpdateError>> + Send + '_>>
        {
            Box::pin(async move {
                self.runs
                    .pop_front()
                    .unwrap_or(Err(WorkflowRunUpdateError::Closed))
            })
        }
    }

    fn test_notifications() -> (WorkflowRunNotifications, mpsc::Receiver<OutgoingEnvelope>) {
        let (tx, rx) = mpsc::channel::<OutgoingEnvelope>(8);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            codex_analytics::AnalyticsEventsClient::disabled(),
        ));
        (WorkflowRunNotifications::new(outgoing), rx)
    }

    fn test_workflow_run(run_id: &str, status: WorkflowRunStatus) -> WorkflowRun {
        WorkflowRun {
            run_id: run_id.to_string(),
            workflow: WorkflowSummary {
                id: "workflow".to_string(),
                name: "Workflow".to_string(),
                description: "A workflow".to_string(),
                source: WorkflowSource::Project,
                path: "/tmp/workflow".to_string(),
                entry: "WORKFLOW.md".to_string(),
                version: Some("1".to_string()),
                when_to_use: vec!["test".to_string()],
                inputs: BTreeMap::new(),
                instructions: "instructions".to_string(),
            },
            status,
            runner_status: "runner".to_string(),
            inputs: json!({ "input": true }),
            created_at: 1,
            updated_at: 2,
            revision: 3,
            message: "message".to_string(),
            abort_reason: None,
            bindings: BTreeMap::new(),
            output: None,
            error: None,
            snapshot_path: None,
        }
    }

    async fn recv_workflow_run_notification(
        rx: &mut mpsc::Receiver<OutgoingEnvelope>,
    ) -> ApiWorkflowRun {
        let envelope = rx.recv().await.expect("expected workflow notification");
        let OutgoingEnvelope::Broadcast { message } = envelope else {
            panic!("expected broadcast workflow notification");
        };
        let OutgoingMessage::AppServerNotification(ServerNotification::WorkflowRunUpdated(
            notification,
        )) = message
        else {
            panic!("expected workflow run updated notification");
        };
        notification.run
    }

    #[test]
    fn workflow_run_terminal_notification_status_excludes_abort() {
        assert!(is_terminal_workflow_run_notification_status(
            WorkflowRunStatus::Completed
        ));
        assert!(is_terminal_workflow_run_notification_status(
            WorkflowRunStatus::Failed
        ));
        assert!(!is_terminal_workflow_run_notification_status(
            WorkflowRunStatus::Running
        ));
        assert!(!is_terminal_workflow_run_notification_status(
            WorkflowRunStatus::Aborted
        ));
    }

    #[test]
    fn workflow_run_projection_preserves_status_response_shape() {
        let mut run = test_workflow_run("run-1", WorkflowRunStatus::Completed);
        run.abort_reason = Some("stopped".to_string());
        run.output = Some(json!({ "ok": true }));
        run.error = Some("error".to_string());
        run.snapshot_path = Some("/tmp/snapshot.json".to_string());

        let response = map_workflow_run(run);

        assert_eq!(response.run_id, "run-1");
        assert_eq!(response.workflow.id, "workflow");
        assert_eq!(response.status, ApiWorkflowRunStatus::Completed);
        assert_eq!(response.runner_status, "runner");
        assert_eq!(response.inputs, json!({ "input": true }));
        assert_eq!(response.abort_reason.as_deref(), Some("stopped"));
        assert_eq!(response.output, Some(json!({ "ok": true })));
        assert_eq!(response.error.as_deref(), Some("error"));
        assert_eq!(
            response.snapshot_path.as_deref(),
            Some("/tmp/snapshot.json")
        );
    }

    #[tokio::test]
    async fn started_run_notification_emits_current_run_before_terminal_update() {
        let (notifications, mut rx) = test_notifications();
        let running = test_workflow_run("run-1", WorkflowRunStatus::Running);
        let completed = test_workflow_run("run-1", WorkflowRunStatus::Completed);

        let response = notifications
            .finish_started_run(
                running.clone(),
                Box::new(TestWorkflowRunUpdates::new(vec![Ok(completed.clone())])),
            )
            .await;

        assert_eq!(response.run_id, "run-1");
        assert_eq!(response.status, ApiWorkflowRunStatus::Running);
        assert_eq!(
            recv_workflow_run_notification(&mut rx).await,
            map_workflow_run(running)
        );
        assert_eq!(
            recv_workflow_run_notification(&mut rx).await,
            map_workflow_run(completed)
        );
    }

    #[tokio::test]
    async fn started_run_terminal_subscription_filters_run_id_and_non_terminal_updates() {
        let (notifications, mut rx) = test_notifications();
        let running = test_workflow_run("run-1", WorkflowRunStatus::Running);
        let other_completed = test_workflow_run("run-2", WorkflowRunStatus::Completed);
        let same_running = test_workflow_run("run-1", WorkflowRunStatus::Running);
        let failed = test_workflow_run("run-1", WorkflowRunStatus::Failed);

        notifications
            .finish_started_run(
                running,
                Box::new(TestWorkflowRunUpdates::new(vec![
                    Ok(other_completed),
                    Ok(same_running),
                    Err(WorkflowRunUpdateError::Lagged(1)),
                    Ok(failed.clone()),
                ])),
            )
            .await;

        let immediate = recv_workflow_run_notification(&mut rx).await;
        assert_eq!(immediate.run_id, "run-1");
        assert_eq!(immediate.status, ApiWorkflowRunStatus::Running);
        assert_eq!(
            recv_workflow_run_notification(&mut rx).await,
            map_workflow_run(failed)
        );
    }

    #[tokio::test]
    async fn direct_run_update_emits_once_without_terminal_subscription() {
        let (notifications, mut rx) = test_notifications();
        let aborted = test_workflow_run("run-1", WorkflowRunStatus::Aborted);

        let response = notifications.finish_run_update(aborted.clone()).await;

        assert_eq!(response.status, ApiWorkflowRunStatus::Aborted);
        assert_eq!(
            recv_workflow_run_notification(&mut rx).await,
            map_workflow_run(aborted)
        );
        assert!(rx.try_recv().is_err());
    }
}
