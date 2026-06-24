use crate::function_tool::FunctionCallError;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::context::SharedTurnDiffTracker;
use crate::tools::handlers::CoreToolDomainHost;
use crate::workflow_runs::WorkflowAgentBinding;
use crate::workflow_runs::WorkflowRun;
use crate::workflow_runs::WorkflowRunController;
use crate::workflow_runs::WorkflowRuntimeBridge;
use crate::workflow_runs::WorkflowRuntimeError;
use crate::workflow_runs::WorkflowRuntimeRequest;
use crate::workflows::WorkflowRegistry;
use crate::workflows::load_workflow_registry;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::WorkflowRunProgressEvent;
use codex_protocol::models::WorkflowRunProgressKind;
use codex_tool_runtime_api::SpawnAgentToolResult;
use codex_tool_runtime_api::WorkflowToolHost;
use codex_tool_runtime_api::run_followup_task_tool;
use codex_tool_runtime_api::run_spawn_agent_tool;
use codex_tool_runtime_api::run_wait_agent_tool;
use codex_tool_runtime_api::spawn_agent_request_from_arguments;
use codex_workflow_api::workflow_followup_task_tool_call;
use codex_workflow_api::workflow_spawn_agent_tool_call;
use codex_workflow_api::workflow_tool_call_id;
use codex_workflow_api::workflow_wait_agent_tool_call;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

struct CodexWorkflowRuntimeBridge {
    session: Arc<Session>,
    turn: Arc<TurnContext>,
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
        let arguments = serde_json::to_string(&tool_call.arguments)
            .map_err(|err| WorkflowRuntimeError::invalid_request(err.to_string()))?;
        let spawn_request = spawn_agent_request_from_arguments(&arguments)
            .map_err(runtime_error_from_tool_error)?;
        let host = crate::tools::handlers::core_tool_domain_host();
        let result = run_spawn_agent_tool(
            &host,
            Arc::clone(&self.session),
            Arc::clone(&self.turn),
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
        let host = crate::tools::handlers::core_tool_domain_host();
        run_followup_task_tool(
            &host,
            Arc::clone(&self.session),
            Arc::clone(&self.turn),
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
        let host = crate::tools::handlers::core_tool_domain_host();
        let result = run_wait_agent_tool(
            &host,
            Arc::clone(&self.session),
            Arc::clone(&self.turn),
            workflow_tool_call_id(&request, "wait_agent"),
            tool_call.target,
        )
        .await
        .map_err(runtime_error_from_tool_error)?;
        serde_json::to_value(result)
            .map_err(|err| WorkflowRuntimeError::invalid_request(err.to_string()))
    }
}

impl WorkflowToolHost for CoreToolDomainHost {
    type Session = Arc<Session>;
    type Turn = Arc<TurnContext>;
    type Tracker = SharedTurnDiffTracker;
    type DiffContext = TurnContext;

    fn load_workflow_registry(&self, turn: &Self::Turn) -> WorkflowRegistry {
        load_workflow_registry(&turn.config)
    }

    fn workflow_run_controller(&self, session: &Self::Session) -> Arc<dyn WorkflowRunController> {
        Arc::clone(&session.workflow_runs)
    }

    fn create_workflow_runtime_bridge(
        &self,
        session: Self::Session,
        turn: Self::Turn,
        _cancellation_token: CancellationToken,
        _tracker: Self::Tracker,
    ) -> Arc<dyn WorkflowRuntimeBridge> {
        Arc::new(CodexWorkflowRuntimeBridge { session, turn })
    }

    async fn record_workflow_progress(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
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
        session
            .record_model_items_and_emit_display_events(turn, std::slice::from_ref(&item))
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
