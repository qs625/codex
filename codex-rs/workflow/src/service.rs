use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Weak;

use protocol::models::ResponseItem;
use protocol::models::WorkflowRunProgressEvent;
use protocol::models::WorkflowRunProgressKind;
use serde::Deserialize;
use serde_json::Value;
use thread_service_api::ThreadAgentMode;
use thread_service_api::ThreadServiceApi;
use thread_service_api::ThreadSpawnAgentForkMode;
use thread_service_api::ThreadSpawnAgentRequest;
use thread_service_api::ThreadSpawnAgentResult;
use thread_service_api::ThreadTurnCapability;
use tool_types::FunctionCallError;

use crate::workflow_runs::WorkflowRunManager;

use codex_workflow_api::WorkflowAbortArgs;
use codex_workflow_api::WorkflowAgentBinding;
use codex_workflow_api::WorkflowApi;
use codex_workflow_api::WorkflowDescribeArgs;
use codex_workflow_api::WorkflowDetails;
use codex_workflow_api::WorkflowDiscoveryContext;
use codex_workflow_api::WorkflowExecutionContext;
use codex_workflow_api::WorkflowProgressFuture;
use codex_workflow_api::WorkflowProgressSink;
use codex_workflow_api::WorkflowRegistry;
use codex_workflow_api::WorkflowResumeArgs;
use codex_workflow_api::WorkflowRunController;
use codex_workflow_api::WorkflowRunFuture;
use codex_workflow_api::WorkflowRunStatus;
use codex_workflow_api::WorkflowRunUpdateError;
use codex_workflow_api::WorkflowRunUpdateReceiver;
use codex_workflow_api::WorkflowRuntimeBridge;
use codex_workflow_api::WorkflowRuntimeError;
use codex_workflow_api::WorkflowRuntimeRequest;
use codex_workflow_api::WorkflowStartArgs;
use codex_workflow_api::WorkflowStatusArgs;
use codex_workflow_api::workflow_followup_task_tool_call;
use codex_workflow_api::workflow_spawn_agent_tool_call;
use codex_workflow_api::workflow_tool_call_id;
use codex_workflow_api::workflow_wait_agent_tool_call;

pub struct WorkflowService {
    workflow_runs: Arc<dyn WorkflowRunController>,
    thread_service_api: Weak<dyn ThreadServiceApi>,
}

impl WorkflowService {
    pub fn new(
        codex_home: impl Into<PathBuf>,
        thread_service_api: Weak<dyn ThreadServiceApi>,
    ) -> Self {
        Self {
            workflow_runs: Arc::new(WorkflowRunManager::new(codex_home)),
            thread_service_api,
        }
    }

    pub fn with_run_manager(
        workflow_runs: Arc<WorkflowRunManager>,
        thread_service_api: Weak<dyn ThreadServiceApi>,
    ) -> Self {
        Self {
            workflow_runs,
            thread_service_api,
        }
    }

    pub fn with_run_controller(
        workflow_runs: Arc<dyn WorkflowRunController>,
        thread_service_api: Weak<dyn ThreadServiceApi>,
    ) -> Self {
        Self {
            workflow_runs,
            thread_service_api,
        }
    }

    fn thread_service_api(&self) -> Result<Weak<dyn ThreadServiceApi>, String> {
        Ok(self.thread_service_api.clone())
    }
}

impl WorkflowApi for WorkflowService {
    fn subscribe_workflow_updates(&self) -> Box<dyn WorkflowRunUpdateReceiver> {
        self.workflow_runs.subscribe()
    }

    fn list_workflows<'a>(
        &'a self,
        discovery: WorkflowDiscoveryContext,
    ) -> Pin<Box<dyn Future<Output = Result<WorkflowRegistry, String>> + Send + 'a>> {
        Box::pin(async move { Ok(load_registry(&discovery)) })
    }

    fn describe_workflow<'a>(
        &'a self,
        discovery: WorkflowDiscoveryContext,
        args: WorkflowDescribeArgs,
    ) -> Pin<Box<dyn Future<Output = Result<WorkflowDetails, String>> + Send + 'a>> {
        Box::pin(async move {
            let workflow = args.workflow().map(str::to_string)?;
            load_registry(&discovery).details(workflow.as_str())
        })
    }

    fn start_workflow<'a>(
        &'a self,
        context: WorkflowExecutionContext,
        args: WorkflowStartArgs,
    ) -> WorkflowRunFuture<'a> {
        Box::pin(async move {
            let workflow_id = args.workflow().map(str::to_string)?;
            let registry = load_registry(context.discovery());
            let updates = self.workflow_runs.subscribe();
            let thread_service_api = self.thread_service_api()?;
            let bridge = Arc::new(ThreadWorkflowRuntimeBridge::new(
                thread_service_api.clone(),
                context.turn(),
            ));
            let run = self
                .workflow_runs
                .start_with_bridge(
                    &registry,
                    &workflow_id,
                    args.inputs.unwrap_or_default(),
                    bridge,
                )
                .await?;
            let progress_sink = ThreadWorkflowProgressSink::new(thread_service_api, context.turn());
            progress_sink
                .record_workflow_progress(
                    &run.run_id,
                    &run.workflow.id,
                    serde_json::to_value(run.status).unwrap_or(Value::Null),
                    Some(run.runner_status.clone()),
                    WorkflowRunProgressKind::Started,
                    Some(run.message.clone()),
                    run.updated_at,
                )
                .await;
            record_terminal_workflow_progress(progress_sink, updates, run.run_id.clone());
            Ok(run)
        })
    }

    fn workflow_status<'a>(&'a self, args: WorkflowStatusArgs) -> WorkflowRunFuture<'a> {
        Box::pin(async move {
            let run_id = args.run_id()?;
            self.workflow_runs.status(run_id).await
        })
    }

    fn resume_workflow<'a>(
        &'a self,
        context: WorkflowExecutionContext,
        args: WorkflowResumeArgs,
    ) -> WorkflowRunFuture<'a> {
        Box::pin(async move {
            let run_id = args.run_id().map(str::to_string)?;
            let updates = self.workflow_runs.subscribe();
            let thread_service_api = self.thread_service_api()?;
            let bridge = Arc::new(ThreadWorkflowRuntimeBridge::new(
                thread_service_api.clone(),
                context.turn(),
            ));
            let run = self
                .workflow_runs
                .resume_with_bridge(&run_id, args.inputs, bridge)
                .await?;
            let progress_sink = ThreadWorkflowProgressSink::new(thread_service_api, context.turn());
            progress_sink
                .record_workflow_progress(
                    &run.run_id,
                    &run.workflow.id,
                    serde_json::to_value(run.status).unwrap_or(Value::Null),
                    Some(run.runner_status.clone()),
                    WorkflowRunProgressKind::Resumed,
                    Some(run.message.clone()),
                    run.updated_at,
                )
                .await;
            record_terminal_workflow_progress(progress_sink, updates, run.run_id.clone());
            Ok(run)
        })
    }

    fn abort_workflow<'a>(
        &'a self,
        context: WorkflowExecutionContext,
        args: WorkflowAbortArgs,
    ) -> WorkflowRunFuture<'a> {
        Box::pin(async move {
            let run_id = args.run_id().map(str::to_string)?;
            let run = self.workflow_runs.abort(&run_id, args.reason).await?;
            ThreadWorkflowProgressSink::new(self.thread_service_api()?, context.turn())
                .record_workflow_progress(
                    &run.run_id,
                    &run.workflow.id,
                    serde_json::to_value(run.status).unwrap_or(Value::Null),
                    Some(run.runner_status.clone()),
                    WorkflowRunProgressKind::Aborted,
                    Some(run.message.clone()),
                    run.updated_at,
                )
                .await;
            Ok(run)
        })
    }
}

pub fn load_registry(context: &WorkflowDiscoveryContext) -> WorkflowRegistry {
    codex_workflow_api::load_workflow_registry(context)
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
                progress_sink
                    .record_workflow_progress(
                        &run.run_id,
                        &run.workflow.id,
                        serde_json::to_value(run.status).unwrap_or(Value::Null),
                        Some(run.runner_status.clone()),
                        kind,
                        Some(run.message.clone()),
                        run.updated_at,
                    )
                    .await;
                break;
            }
        }
    });
}

struct ThreadWorkflowRuntimeBridge {
    thread_service_api: Weak<dyn ThreadServiceApi>,
    turn: Option<Arc<dyn ThreadTurnCapability>>,
}

impl ThreadWorkflowRuntimeBridge {
    fn new(
        thread_service_api: Weak<dyn ThreadServiceApi>,
        turn: Option<Arc<dyn ThreadTurnCapability>>,
    ) -> Self {
        Self {
            thread_service_api,
            turn,
        }
    }
}

impl WorkflowRuntimeBridge for ThreadWorkflowRuntimeBridge {
    fn call(
        &self,
        request: WorkflowRuntimeRequest,
    ) -> Pin<Box<dyn Future<Output = Result<Value, WorkflowRuntimeError>> + Send + '_>> {
        Box::pin(async move {
            let Some(thread_service_api) = self.thread_service_api.upgrade() else {
                return Err(WorkflowRuntimeError::unsupported(
                    "workflow thread service api is unavailable",
                ));
            };
            let Some(turn) = self.turn.clone() else {
                return Err(WorkflowRuntimeError::unsupported(
                    "workflow execution is not bound to an active thread turn",
                ));
            };
            match request.method.as_str() {
                "agent.spawn" => {
                    let tool_call = workflow_spawn_agent_tool_call(&request)?;
                    let spawn_request = workflow_spawn_agent_request(tool_call.arguments.clone())
                        .map_err(runtime_error_from_tool_error)?;
                    let result = thread_service_api
                        .spawn_agent(
                            Arc::clone(&turn),
                            workflow_tool_call_id(&request, "spawn_agent"),
                            spawn_request,
                        )
                        .await
                        .map_err(runtime_error_from_tool_error)?;
                    let agent_path = match result {
                        ThreadSpawnAgentResult::WithNickname { task_name, .. }
                        | ThreadSpawnAgentResult::HiddenMetadata { task_name } => task_name,
                    };
                    serde_json::to_value(WorkflowAgentBinding {
                        stage_id: Some(tool_call.agent_id.clone()),
                        agent_id: tool_call.agent_id,
                        agent_path,
                        workflow_id: Some(request.workflow_id),
                        run_id: Some(request.run_id),
                        thread_id: None,
                        status: None,
                        options: tool_call.options,
                    })
                    .map_err(|err| WorkflowRuntimeError::invalid_request(err.to_string()))
                }
                "agent.followup" => {
                    let tool_call = workflow_followup_task_tool_call(&request)?;
                    thread_service_api
                        .followup_task(
                            Arc::clone(&turn),
                            workflow_tool_call_id(&request, "followup_task"),
                            tool_call.target,
                            tool_call.message,
                        )
                        .await
                        .map_err(runtime_error_from_tool_error)?;
                    Ok(serde_json::json!({ "ok": true }))
                }
                "agent.wait" => {
                    let _tool_call = workflow_wait_agent_tool_call(&request)?;
                    let result = thread_service_api
                        .poll_event(
                            Arc::clone(&turn),
                            thread_service_api::ThreadPollEventRequest {
                                initial_timeout_ms: None,
                                hard_cap_timeout_ms: None,
                            },
                        )
                        .await
                        .map_err(runtime_error_from_tool_error)?;
                    serde_json::to_value(result)
                        .map_err(|err| WorkflowRuntimeError::invalid_request(err.to_string()))
                }
                "shell.exec" => Err(WorkflowRuntimeError::unsupported(
                    "wf.shell is not connected to exec_command in this phase; use an agent to request shell work",
                )),
                method => Err(WorkflowRuntimeError::unsupported(format!(
                    "unsupported workflow runtime method `{method}`"
                ))),
            }
        })
    }
}

struct ThreadWorkflowProgressSink {
    thread_service_api: Weak<dyn ThreadServiceApi>,
    turn: Option<Arc<dyn ThreadTurnCapability>>,
}

impl ThreadWorkflowProgressSink {
    fn new(
        thread_service_api: Weak<dyn ThreadServiceApi>,
        turn: Option<Arc<dyn ThreadTurnCapability>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            thread_service_api,
            turn,
        })
    }
}

impl WorkflowProgressSink for ThreadWorkflowProgressSink {
    fn record_workflow_progress<'a>(
        &'a self,
        run_id: &'a str,
        workflow_id: &'a str,
        status: Value,
        runner_status: Option<String>,
        kind: WorkflowRunProgressKind,
        message: Option<String>,
        updated_at: i64,
    ) -> WorkflowProgressFuture<'a> {
        Box::pin(async move {
            let Some(thread_service_api) = self.thread_service_api.upgrade() else {
                return;
            };
            let Some(turn) = self.turn.clone() else {
                return;
            };
            let item = ResponseItem::WorkflowRunProgress {
                id: None,
                event: WorkflowRunProgressEvent {
                    run_id: run_id.to_string(),
                    workflow_id: workflow_id.to_string(),
                    status,
                    kind,
                    message: message.unwrap_or_default(),
                    runner_status: runner_status.unwrap_or_default(),
                    updated_at,
                },
            };
            let _ = thread_service_api
                .record_model_items_and_emit_display_events(turn, vec![item])
                .await;
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowSpawnAgentArgs {
    message: String,
    task_name: String,
    agent_type: Option<String>,
    cwd: Option<codex_utils_absolute_path::AbsolutePathBuf>,
    model: Option<String>,
    reasoning_effort: Option<protocol::openai_models::ReasoningEffort>,
    service_tier: Option<String>,
    agent_mode: Option<ThreadAgentMode>,
    fork_turns: Option<String>,
    fork_context: Option<bool>,
}

impl WorkflowSpawnAgentArgs {
    fn into_request(self) -> Result<ThreadSpawnAgentRequest, FunctionCallError> {
        let fork_mode = self.fork_mode()?;
        Ok(ThreadSpawnAgentRequest {
            message: self.message,
            task_name: self.task_name,
            agent_type: self.agent_type,
            cwd: self.cwd,
            model: self.model,
            reasoning_effort: self.reasoning_effort,
            service_tier: self.service_tier,
            agent_mode: self.agent_mode,
            fork_mode,
        })
    }

    fn fork_mode(&self) -> Result<Option<ThreadSpawnAgentForkMode>, FunctionCallError> {
        if self.fork_context.is_some() {
            return Err(FunctionCallError::RespondToModel(
                "fork_context is not supported in MultiAgentV2; use fork_turns instead".to_string(),
            ));
        }

        let fork_turns = self
            .fork_turns
            .as_deref()
            .map(str::trim)
            .filter(|fork_turns| !fork_turns.is_empty())
            .unwrap_or("all");

        if fork_turns.eq_ignore_ascii_case("none") {
            return Ok(None);
        }
        if fork_turns.eq_ignore_ascii_case("all") {
            return Ok(Some(ThreadSpawnAgentForkMode::FullHistory));
        }

        let last_n_turns = fork_turns.parse::<usize>().map_err(|_| {
            FunctionCallError::RespondToModel(
                "fork_turns must be `none`, `all`, or a positive integer string".to_string(),
            )
        })?;
        if last_n_turns == 0 {
            return Err(FunctionCallError::RespondToModel(
                "fork_turns must be `none`, `all`, or a positive integer string".to_string(),
            ));
        }

        Ok(Some(ThreadSpawnAgentForkMode::LastNTurns { last_n_turns }))
    }
}

fn workflow_spawn_agent_request(
    arguments: Value,
) -> Result<ThreadSpawnAgentRequest, FunctionCallError> {
    serde_json::from_value::<WorkflowSpawnAgentArgs>(arguments)
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "failed to parse workflow spawn_agent arguments: {err}"
            ))
        })?
        .into_request()
}

fn runtime_error_from_tool_error(error: FunctionCallError) -> WorkflowRuntimeError {
    match error {
        FunctionCallError::RespondToModel(message) => {
            WorkflowRuntimeError::invalid_request(message)
        }
        FunctionCallError::Fatal(message) => WorkflowRuntimeError {
            code: "runtime_error".to_string(),
            message,
        },
    }
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
