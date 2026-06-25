use crate::function_tool::FunctionCallError;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::context::SharedTurnDiffTracker;
use crate::workflow_runs::WorkflowAgentBinding;
use crate::workflow_runs::WorkflowRun;
use crate::workflow_runs::WorkflowRuntimeBridge;
use crate::workflow_runs::WorkflowRuntimeError;
use crate::workflow_runs::WorkflowRuntimeRequest;
use codex_agent_runtime::AgentMode;
use codex_agent_runtime::MultiAgentToolSession;
use codex_agent_runtime::SpawnAgentForkMode;
use codex_agent_runtime::SpawnAgentToolRequest;
use codex_agent_runtime::SpawnAgentToolResult;
use codex_agent_runtime::WaitAgentToolResult;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::WorkflowRunProgressEvent;
use codex_protocol::models::WorkflowRunProgressKind;
use codex_protocol::openai_models::ReasoningEffort;
use codex_session_api::SessionWorkflowCaller;
use codex_session_api::SessionWorkflowTurn;
use codex_tool_runtime_api::WorkflowAgentToolRuntime;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_workflow_api::WorkflowRegistry;
use codex_workflow_api::WorkflowRunController;
use codex_workflow_api::workflow_followup_task_tool_call;
use codex_workflow_api::workflow_spawn_agent_tool_call;
use codex_workflow_api::workflow_tool_call_id;
use codex_workflow_api::workflow_wait_agent_tool_call;
use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

struct CodexWorkflowRuntimeBridge {
    agent_runtime: Arc<dyn WorkflowAgentToolRuntime>,
}

struct CoreWorkflowAgentToolRuntime {
    session: Arc<Session>,
    turn: Arc<TurnContext>,
}

impl CoreWorkflowAgentToolRuntime {
    fn new(session: Arc<Session>, turn: Arc<TurnContext>) -> Self {
        Self { session, turn }
    }
}

impl WorkflowAgentToolRuntime for CoreWorkflowAgentToolRuntime {
    fn spawn_agent(
        &self,
        call_id: String,
        request: SpawnAgentToolRequest,
    ) -> BoxFuture<'_, Result<SpawnAgentToolResult, FunctionCallError>> {
        Box::pin(async move {
            Arc::clone(&self.session)
                .spawn_agent_tool(&self.turn, call_id, request)
                .await
        })
    }

    fn followup_agent(
        &self,
        call_id: String,
        target: String,
        message: String,
    ) -> BoxFuture<'_, Result<(), FunctionCallError>> {
        Box::pin(async move {
            Arc::clone(&self.session)
                .followup_task_tool(&self.turn, call_id, target, message)
                .await
        })
    }

    fn wait_agent(
        &self,
        call_id: String,
        target: String,
    ) -> BoxFuture<'_, Result<WaitAgentToolResult, FunctionCallError>> {
        Box::pin(async move {
            Arc::clone(&self.session)
                .wait_agent_tool(&self.turn, call_id, target)
                .await
        })
    }
}

impl WorkflowRuntimeBridge for CodexWorkflowRuntimeBridge {
    fn call(
        &self,
        request: WorkflowRuntimeRequest,
    ) -> Pin<Box<dyn Future<Output = Result<Value, WorkflowRuntimeError>> + Send + '_>> {
        Box::pin(async move { self.handle_request(request).await })
    }
}

impl CodexWorkflowRuntimeBridge {
    async fn handle_request(
        &self,
        request: WorkflowRuntimeRequest,
    ) -> Result<Value, WorkflowRuntimeError> {
        match request.method.as_str() {
            "agent.spawn" => self.spawn_agent(request).await,
            "agent.followup" => self.followup_agent(request).await,
            "agent.wait" => self.wait_agent(request).await,
            "shell.exec" => Err(WorkflowRuntimeError::unsupported(
                "wf.shell is not connected to exec_command in this phase; use an agent to request shell work",
            )),
            method => Err(WorkflowRuntimeError::unsupported(format!(
                "unsupported workflow runtime method `{method}`"
            ))),
        }
    }

    async fn spawn_agent(
        &self,
        request: WorkflowRuntimeRequest,
    ) -> Result<Value, WorkflowRuntimeError> {
        let tool_call = workflow_spawn_agent_tool_call(&request)?;
        let spawn_request = workflow_spawn_agent_request(tool_call.arguments.clone())
            .map_err(runtime_error_from_tool_error)?;
        let result = self
            .agent_runtime
            .spawn_agent(
                workflow_tool_call_id(&request, "spawn_agent"),
                spawn_request,
            )
            .await
            .map_err(runtime_error_from_tool_error)?;
        let agent_path = match result {
            SpawnAgentToolResult::WithNickname { task_name, .. }
            | SpawnAgentToolResult::HiddenMetadata { task_name } => task_name,
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

    async fn followup_agent(
        &self,
        request: WorkflowRuntimeRequest,
    ) -> Result<Value, WorkflowRuntimeError> {
        let tool_call = workflow_followup_task_tool_call(&request)?;
        self.agent_runtime
            .followup_agent(
                workflow_tool_call_id(&request, "followup_task"),
                tool_call.target,
                tool_call.message,
            )
            .await
            .map_err(runtime_error_from_tool_error)?;
        Ok(serde_json::json!({ "ok": true }))
    }

    async fn wait_agent(
        &self,
        request: WorkflowRuntimeRequest,
    ) -> Result<Value, WorkflowRuntimeError> {
        let tool_call = workflow_wait_agent_tool_call(&request)?;
        let result = self
            .agent_runtime
            .wait_agent(
                workflow_tool_call_id(&request, "wait_agent"),
                tool_call.target,
            )
            .await
            .map_err(runtime_error_from_tool_error)?;
        serde_json::to_value(result)
            .map_err(|err| WorkflowRuntimeError::invalid_request(err.to_string()))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowSpawnAgentArgs {
    message: String,
    task_name: String,
    agent_type: Option<String>,
    cwd: Option<AbsolutePathBuf>,
    model: Option<String>,
    reasoning_effort: Option<ReasoningEffort>,
    service_tier: Option<String>,
    agent_mode: Option<AgentMode>,
    fork_turns: Option<String>,
    fork_context: Option<bool>,
}

impl WorkflowSpawnAgentArgs {
    fn into_request(self) -> Result<SpawnAgentToolRequest, FunctionCallError> {
        let fork_mode = self.fork_mode()?;
        Ok(SpawnAgentToolRequest {
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

    fn fork_mode(&self) -> Result<Option<SpawnAgentForkMode>, FunctionCallError> {
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
            return Ok(Some(SpawnAgentForkMode::FullHistory));
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

        Ok(Some(SpawnAgentForkMode::LastNTurns(last_n_turns)))
    }
}

fn workflow_spawn_agent_request(
    arguments: Value,
) -> Result<SpawnAgentToolRequest, FunctionCallError> {
    serde_json::from_value::<WorkflowSpawnAgentArgs>(arguments)
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "failed to parse workflow spawn_agent arguments: {err}"
            ))
        })?
        .into_request()
}

impl SessionWorkflowTurn for TurnContext {
    fn load_workflow_registry(&self) -> WorkflowRegistry {
        TurnContext::load_workflow_registry(self)
    }
}

impl SessionWorkflowCaller<Arc<TurnContext>, SharedTurnDiffTracker> for Session {
    fn workflow_run_controller(self: Arc<Self>) -> Arc<dyn WorkflowRunController> {
        Arc::clone(&self.workflow_runs)
    }

    fn create_workflow_runtime_bridge(
        self: Arc<Self>,
        turn: Arc<TurnContext>,
        _cancellation_token: CancellationToken,
        _tracker: SharedTurnDiffTracker,
    ) -> Arc<dyn WorkflowRuntimeBridge> {
        Arc::new(CodexWorkflowRuntimeBridge {
            agent_runtime: Arc::new(CoreWorkflowAgentToolRuntime::new(self, turn)),
        })
    }

    async fn record_workflow_progress(
        self: Arc<Self>,
        turn: &Arc<TurnContext>,
        run: &WorkflowRun,
        kind: WorkflowRunProgressKind,
    ) {
        let item = ResponseItem::WorkflowRunProgress {
            id: None,
            event: WorkflowRunProgressEvent {
                run_id: run.run_id.clone(),
                workflow_id: run.workflow.id.clone(),
                status: serde_json::to_value(run.status).unwrap_or(Value::Null),
                runner_status: run.runner_status.clone(),
                kind,
                message: run.message.clone(),
                updated_at: run.updated_at,
            },
        };
        self.record_model_items_and_emit_display_events(turn, std::slice::from_ref(&item))
            .await;
    }
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
