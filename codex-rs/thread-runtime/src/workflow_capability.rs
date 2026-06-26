use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::workflow_runs::WorkflowAgentBinding;
use crate::workflow_runs::WorkflowRun;
use crate::workflow_runs::WorkflowRuntimeError;
use crate::workflow_runs::WorkflowRuntimeRequest;
use codex_agent_runtime::AgentMode;
use codex_agent_runtime::MultiAgentToolSession;
use codex_agent_runtime::SpawnAgentForkMode;
use codex_agent_runtime::SpawnAgentToolRequest;
use codex_agent_runtime::SpawnAgentToolResult;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::WorkflowRunProgressEvent;
use codex_protocol::models::WorkflowRunProgressKind;
use codex_protocol::openai_models::ReasoningEffort;
use codex_tool_types::FunctionCallError;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_workflow_api::WorkflowCapability;
use codex_workflow_api::WorkflowProgressFuture;
use codex_workflow_api::WorkflowProgressSink;
use codex_workflow_api::WorkflowRunController;
use codex_workflow_api::WorkflowRuntimeBridge;
use codex_workflow_api::workflow_followup_task_tool_call;
use codex_workflow_api::workflow_spawn_agent_tool_call;
use codex_workflow_api::workflow_tool_call_id;
use codex_workflow_api::workflow_wait_agent_tool_call;
use serde::Deserialize;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

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

async fn handle_workflow_runtime_request(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    request: WorkflowRuntimeRequest,
) -> Result<Value, WorkflowRuntimeError> {
    match request.method.as_str() {
        "agent.spawn" => workflow_spawn_agent(session, turn, request).await,
        "agent.followup" => workflow_followup_agent(session, turn, request).await,
        "agent.wait" => workflow_wait_agent(session, turn, request).await,
        "shell.exec" => Err(WorkflowRuntimeError::unsupported(
            "wf.shell is not connected to exec_command in this phase; use an agent to request shell work",
        )),
        method => Err(WorkflowRuntimeError::unsupported(format!(
            "unsupported workflow runtime method `{method}`"
        ))),
    }
}

async fn workflow_spawn_agent(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    request: WorkflowRuntimeRequest,
) -> Result<Value, WorkflowRuntimeError> {
    let tool_call = workflow_spawn_agent_tool_call(&request)?;
    let spawn_request = workflow_spawn_agent_request(tool_call.arguments.clone())
        .map_err(runtime_error_from_tool_error)?;
    let result = session
        .spawn_agent_tool(&turn, workflow_tool_call_id(&request, "spawn_agent"), spawn_request)
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

async fn workflow_followup_agent(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    request: WorkflowRuntimeRequest,
) -> Result<Value, WorkflowRuntimeError> {
    let tool_call = workflow_followup_task_tool_call(&request)?;
    session
        .followup_task_tool(
            &turn,
            workflow_tool_call_id(&request, "followup_task"),
            tool_call.target,
            tool_call.message,
        )
        .await
        .map_err(runtime_error_from_tool_error)?;
    Ok(serde_json::json!({ "ok": true }))
}

async fn workflow_wait_agent(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    request: WorkflowRuntimeRequest,
) -> Result<Value, WorkflowRuntimeError> {
    let tool_call = workflow_wait_agent_tool_call(&request)?;
    let result = session
        .wait_agent_tool(
            &turn,
            workflow_tool_call_id(&request, "wait_agent"),
            tool_call.target,
        )
        .await
        .map_err(runtime_error_from_tool_error)?;
    serde_json::to_value(result)
        .map_err(|err| WorkflowRuntimeError::invalid_request(err.to_string()))
}

impl WorkflowCapability for TurnContext {
    fn load_workflow_registry(&self) -> codex_workflow_api::WorkflowRegistry {
        TurnContext::load_workflow_registry(self)
    }

    fn workflow_run_controller(&self) -> Arc<dyn WorkflowRunController> {
        Arc::clone(&self.workflow_session().workflow_runs)
    }

    fn create_workflow_runtime_bridge(&self) -> Arc<dyn WorkflowRuntimeBridge> {
        Arc::new(TurnWorkflowRuntimeBridge {
            session: Arc::clone(&self.workflow_session()),
            turn: Arc::new(self.clone_for_workflow_bridge()),
        })
    }

    fn workflow_progress_sink(&self) -> Arc<dyn WorkflowProgressSink> {
        Arc::new(TurnWorkflowProgressSink {
            session: Arc::clone(&self.workflow_session()),
            turn: Arc::new(self.clone_for_workflow_bridge()),
        })
    }
}

struct TurnWorkflowProgressSink {
    session: Arc<Session>,
    turn: Arc<TurnContext>,
}

impl WorkflowProgressSink for TurnWorkflowProgressSink {
    fn record_workflow_progress<'a>(
        &'a self,
        run: &'a WorkflowRun,
        kind: WorkflowRunProgressKind,
    ) -> WorkflowProgressFuture<'a> {
        let session = Arc::clone(&self.session);
        let turn = Arc::clone(&self.turn);
        Box::pin(async move {
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
                .record_model_items_and_emit_display_events(&turn, std::slice::from_ref(&item))
                .await;
        })
    }
}

struct TurnWorkflowRuntimeBridge {
    session: Arc<Session>,
    turn: Arc<TurnContext>,
}

impl codex_workflow_api::WorkflowRuntimeBridge for TurnWorkflowRuntimeBridge {
    fn call(
        &self,
        request: WorkflowRuntimeRequest,
    ) -> Pin<Box<dyn Future<Output = Result<Value, WorkflowRuntimeError>> + Send + '_>> {
        let session = Arc::clone(&self.session);
        let turn = Arc::clone(&self.turn);
        Box::pin(async move { handle_workflow_runtime_request(session, turn, request).await })
    }
}

impl TurnContext {
    fn workflow_session(&self) -> Arc<Session> {
        self.session
            .upgrade()
            .expect("workflow capability requires a live Session backing the TurnContext")
    }

    fn clone_for_workflow_bridge(&self) -> TurnContext {
        TurnContext {
            session: self.session.clone(),
            self_weak: std::sync::OnceLock::new(),
            sub_id: self.sub_id.clone(),
            trace_id: self.trace_id.clone(),
            realtime_active: self.realtime_active,
            config: Arc::clone(&self.config),
            auth_runtime: self.auth_runtime.clone(),
            model_info: self.model_info.clone(),
            session_telemetry: self.session_telemetry.clone(),
            provider: Arc::clone(&self.provider),
            reasoning_effort: self.reasoning_effort.clone(),
            reasoning_summary: self.reasoning_summary.clone(),
            session_source: self.session_source.clone(),
            thread_source: self.thread_source.clone(),
            environments: self.environments.clone(),
            #[allow(deprecated)]
            cwd: self.cwd.clone(),
            current_date: self.current_date.clone(),
            timezone: self.timezone.clone(),
            app_server_client_name: self.app_server_client_name.clone(),
            developer_instructions: self.developer_instructions.clone(),
            compact_prompt: self.compact_prompt.clone(),
            user_instructions: self.user_instructions.clone(),
            collaboration_mode: self.collaboration_mode.clone(),
            personality: self.personality.clone(),
            approval_policy: self.approval_policy.clone(),
            permission_profile: self.permission_profile.clone(),
            network: self.network.clone(),
            windows_sandbox_level: self.windows_sandbox_level,
            shell_environment_policy: self.shell_environment_policy.clone(),
            tools_config: self.tools_config.clone(),
            features: self.features.clone(),
            ghost_snapshot: self.ghost_snapshot.clone(),
            final_output_json_schema: self.final_output_json_schema.clone(),
            codex_self_exe: self.codex_self_exe.clone(),
            codex_linux_sandbox_exe: self.codex_linux_sandbox_exe.clone(),
            truncation_policy: self.truncation_policy.clone(),
            dynamic_tools: self.dynamic_tools.clone(),
            turn_metadata_state: Arc::clone(&self.turn_metadata_state),
            extension_data: Arc::clone(&self.extension_data),
            turn_skills: self.turn_skills.clone(),
            turn_timing_state: Arc::clone(&self.turn_timing_state),
            server_model_warning_emitted: AtomicBool::new(
                self.server_model_warning_emitted.load(std::sync::atomic::Ordering::SeqCst),
            ),
            model_verification_emitted: AtomicBool::new(
                self.model_verification_emitted.load(std::sync::atomic::Ordering::SeqCst),
            ),
        }
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
