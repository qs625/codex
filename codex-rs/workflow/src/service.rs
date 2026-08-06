use std::collections::VecDeque;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Weak;

#[cfg(test)]
use protocol::AgentPath;
use protocol::models::ResponseItem;
use protocol::models::WorkflowRunProgressEvent;
use protocol::models::WorkflowRunProgressKind;
use protocol::protocol::AgentStatus;
use protocol::protocol::InterAgentCommunication;
use protocol::protocol::InterAgentOperation;
use serde::Deserialize;
use serde_json::Value;
use thread_service_api::NativeAgentRuntime;
use thread_service_api::NativeTurnEventRuntime;
use thread_service_api::ThreadFollowupTaskInput;
use thread_service_api::ThreadPollEvent;
use thread_service_api::ThreadPollEventRequest;
use thread_service_api::ThreadPollEventResult;
use thread_service_api::ThreadServiceFuture;
use thread_service_api::ThreadSpawnAgentForkMode;
use thread_service_api::ThreadSpawnAgentRequest;
use thread_service_api::ThreadSpawnAgentResult;
use thread_service_api::ThreadTurnCapability;
use tokio::sync::Mutex;
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
use codex_workflow_api::workflow_legacy_agent_wait_tool_call;
use codex_workflow_api::workflow_poll_event_tool_call;
use codex_workflow_api::workflow_spawn_agent_tool_call;
use codex_workflow_api::workflow_tool_call_id;

pub struct WorkflowService {
    workflow_runs: Arc<dyn WorkflowRunController>,
    thread_runtime: Weak<dyn WorkflowThreadRuntime>,
}

/// Native agent operations used by workflow runner scripts.
///
/// Workflow execution is currently exposed only for the native provider, so
/// these methods intentionally keep native `spawn_agent` / `followup_task`
/// semantics rather than pretending to be provider-neutral child spawning.
pub trait WorkflowNativeAgentRuntime: Send + Sync + 'static {
    fn spawn_agent<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        request: ThreadSpawnAgentRequest,
    ) -> ThreadServiceFuture<'a, Result<ThreadSpawnAgentResult, FunctionCallError>>;

    fn followup_task<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        target: String,
        input: ThreadFollowupTaskInput,
    ) -> ThreadServiceFuture<'a, Result<(), FunctionCallError>>;
}

/// Native turn-bound poll/progress adapter used by workflow runner scripts.
///
/// `event.poll` and workflow progress still execute against the native turn
/// capability. External provider `poll_external_event` uses a separate
/// thread-id scoped path and does not implement this workflow adapter.
pub trait WorkflowNativeTurnEventRuntime: Send + Sync + 'static {
    fn poll_event<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        request: ThreadPollEventRequest,
    ) -> ThreadServiceFuture<'a, Result<ThreadPollEventResult, FunctionCallError>>;

    fn record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        items: Vec<ResponseItem>,
    ) -> ThreadServiceFuture<'a, Result<(), String>>;
}

pub trait WorkflowThreadRuntime:
    WorkflowNativeAgentRuntime + WorkflowNativeTurnEventRuntime + Send + Sync + 'static
{
}

impl<T> WorkflowThreadRuntime for T where
    T: WorkflowNativeAgentRuntime + WorkflowNativeTurnEventRuntime + Send + Sync + 'static
{
}

impl<T> WorkflowNativeAgentRuntime for T
where
    T: NativeAgentRuntime,
{
    fn spawn_agent<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        request: ThreadSpawnAgentRequest,
    ) -> ThreadServiceFuture<'a, Result<ThreadSpawnAgentResult, FunctionCallError>> {
        NativeAgentRuntime::spawn_agent(self, turn, call_id, request)
    }

    fn followup_task<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        target: String,
        input: ThreadFollowupTaskInput,
    ) -> ThreadServiceFuture<'a, Result<(), FunctionCallError>> {
        NativeAgentRuntime::followup_task(self, turn, call_id, target, input)
    }
}

impl<T> WorkflowNativeTurnEventRuntime for T
where
    T: NativeTurnEventRuntime,
{
    fn poll_event<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        request: ThreadPollEventRequest,
    ) -> ThreadServiceFuture<'a, Result<ThreadPollEventResult, FunctionCallError>> {
        NativeTurnEventRuntime::poll_event(self, turn, request)
    }

    fn record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        items: Vec<ResponseItem>,
    ) -> ThreadServiceFuture<'a, Result<(), String>> {
        NativeTurnEventRuntime::record_model_items_and_emit_display_events(self, turn, items)
    }
}

impl WorkflowService {
    pub fn new(
        codex_home: impl Into<PathBuf>,
        thread_runtime: Weak<dyn WorkflowThreadRuntime>,
    ) -> Self {
        Self {
            workflow_runs: Arc::new(WorkflowRunManager::new(codex_home)),
            thread_runtime,
        }
    }

    pub fn with_run_manager(
        workflow_runs: Arc<WorkflowRunManager>,
        thread_runtime: Weak<dyn WorkflowThreadRuntime>,
    ) -> Self {
        Self {
            workflow_runs,
            thread_runtime,
        }
    }

    pub fn with_run_controller(
        workflow_runs: Arc<dyn WorkflowRunController>,
        thread_runtime: Weak<dyn WorkflowThreadRuntime>,
    ) -> Self {
        Self {
            workflow_runs,
            thread_runtime,
        }
    }

    fn thread_runtime(&self) -> Result<Weak<dyn WorkflowThreadRuntime>, String> {
        Ok(self.thread_runtime.clone())
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
            let thread_runtime = self.thread_runtime()?;
            let bridge = Arc::new(ThreadWorkflowRuntimeBridge::new(
                thread_runtime.clone(),
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
            let progress_sink = ThreadWorkflowProgressSink::new(thread_runtime, context.turn());
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
            let thread_runtime = self.thread_runtime()?;
            let bridge = Arc::new(ThreadWorkflowRuntimeBridge::new(
                thread_runtime.clone(),
                context.turn(),
            ));
            let run = self
                .workflow_runs
                .resume_with_bridge(&run_id, args.inputs, bridge)
                .await?;
            let progress_sink = ThreadWorkflowProgressSink::new(thread_runtime, context.turn());
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
            ThreadWorkflowProgressSink::new(self.thread_runtime()?, context.turn())
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
    thread_runtime: Weak<dyn WorkflowThreadRuntime>,
    turn: Option<Arc<dyn ThreadTurnCapability>>,
    pending_events: Mutex<VecDeque<CachedWorkflowEvent>>,
    consumed_completion_keys: Mutex<HashSet<String>>,
}

impl ThreadWorkflowRuntimeBridge {
    fn new(
        thread_runtime: Weak<dyn WorkflowThreadRuntime>,
        turn: Option<Arc<dyn ThreadTurnCapability>>,
    ) -> Self {
        Self {
            thread_runtime,
            turn,
            pending_events: Mutex::new(VecDeque::new()),
            consumed_completion_keys: Mutex::new(HashSet::new()),
        }
    }
}

#[derive(Debug, Clone)]
struct CachedWorkflowEvent {
    event: ThreadPollEvent,
    completion_key: Option<String>,
}

impl WorkflowRuntimeBridge for ThreadWorkflowRuntimeBridge {
    fn call(
        &self,
        request: WorkflowRuntimeRequest,
    ) -> Pin<Box<dyn Future<Output = Result<Value, WorkflowRuntimeError>> + Send + '_>> {
        Box::pin(async move {
            let Some(thread_runtime) = self.thread_runtime.upgrade() else {
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
                    let result = thread_runtime
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
                    thread_runtime
                        .followup_task(
                            Arc::clone(&turn),
                            workflow_tool_call_id(&request, "followup_task"),
                            tool_call.target,
                            ThreadFollowupTaskInput {
                                message: tool_call.message,
                                content_parts: Vec::new(),
                            },
                        )
                        .await
                        .map_err(runtime_error_from_tool_error)?;
                    Ok(serde_json::json!({ "ok": true }))
                }
                "agent.wait" => {
                    let tool_call = workflow_legacy_agent_wait_tool_call(&request)?;
                    wait_for_workflow_agent(
                        &thread_runtime,
                        Arc::clone(&turn),
                        self,
                        tool_call.agent_id.as_deref(),
                        &tool_call.target,
                    )
                    .await
                }
                "event.poll" => {
                    let _tool_call = workflow_poll_event_tool_call(&request)?;
                    poll_workflow_event(&thread_runtime, Arc::clone(&turn), self).await
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

async fn poll_workflow_event(
    thread_runtime: &Arc<dyn WorkflowThreadRuntime>,
    turn: Arc<dyn ThreadTurnCapability>,
    bridge: &ThreadWorkflowRuntimeBridge,
) -> Result<Value, WorkflowRuntimeError> {
    if let Some(result) = bridge.cached_poll_result().await {
        return serde_json::to_value(result)
            .map_err(|err| WorkflowRuntimeError::invalid_request(err.to_string()));
    }
    let result = thread_runtime
        .poll_event(
            turn,
            ThreadPollEventRequest {
                initial_timeout_ms: None,
                hard_cap_timeout_ms: None,
            },
        )
        .await
        .map_err(runtime_error_from_tool_error)?;
    serde_json::to_value(result)
        .map_err(|err| WorkflowRuntimeError::invalid_request(err.to_string()))
}

async fn wait_for_workflow_agent(
    thread_runtime: &Arc<dyn WorkflowThreadRuntime>,
    turn: Arc<dyn ThreadTurnCapability>,
    bridge: &ThreadWorkflowRuntimeBridge,
    agent_id: Option<&str>,
    target: &str,
) -> Result<Value, WorkflowRuntimeError> {
    loop {
        if let Some(event) = bridge.take_cached_matching_agent_completion(target).await {
            return workflow_agent_wait_result(agent_id, target, event);
        }

        let poll_result = thread_runtime
            .poll_event(
                Arc::clone(&turn),
                ThreadPollEventRequest {
                    initial_timeout_ms: None,
                    hard_cap_timeout_ms: None,
                },
            )
            .await
            .map_err(runtime_error_from_tool_error)?;
        let (matching, pending) = bridge
            .split_wait_events(poll_events_with_primary(poll_result), target)
            .await;
        bridge.push_pending_events(pending).await;
        if let Some(event) = matching {
            return workflow_agent_wait_result(agent_id, target, event);
        }
    }
}

fn split_wait_events_for_bridge(
    events: Vec<ThreadPollEvent>,
    target: &str,
    consumed_completion_keys: &mut HashSet<String>,
) -> (Option<ThreadPollEvent>, Vec<CachedWorkflowEvent>) {
    let mut matching = None;
    let mut pending = Vec::new();
    let mut occurrence_by_base_key = HashMap::<String, usize>::new();
    for event in events {
        let completion_key = completion_event_key(&event, &mut occurrence_by_base_key);
        if let Some(key) = completion_key.as_ref()
            && consumed_completion_keys.contains(key)
        {
            continue;
        }
        if is_target_agent_completion(&event, target) {
            if matching.is_none()
                && let Some(key) = completion_key.as_ref()
            {
                consumed_completion_keys.insert(key.clone());
                matching = Some(event);
                continue;
            }
        }
        pending.push(CachedWorkflowEvent {
            event,
            completion_key,
        });
    }
    (matching, pending)
}

fn poll_events_with_primary(result: ThreadPollEventResult) -> Vec<ThreadPollEvent> {
    let mut events = result.events;
    if let Some(event) = result.event
        && !events.contains(&event)
    {
        events.insert(0, event);
    }
    events
}

fn is_target_agent_completion(event: &ThreadPollEvent, target: &str) -> bool {
    let ThreadPollEvent::InterAgentCommunication { communication } = event else {
        return false;
    };
    communication.author.as_str() == target
        && matches!(
            communication.operation,
            InterAgentOperation::ChildCompletion
        )
}

fn completion_event_key(
    event: &ThreadPollEvent,
    occurrence_by_base_key: &mut HashMap<String, usize>,
) -> Option<String> {
    let ThreadPollEvent::InterAgentCommunication { communication } = event else {
        return None;
    };
    if !matches!(
        communication.operation,
        InterAgentOperation::ChildCompletion
    ) {
        return None;
    }
    let base_key = serde_json::json!({
        "author": &communication.author,
        "senderThreadId": &communication.sender_thread_id,
        "operation": &communication.operation,
        "status": &communication.status,
        "content": &communication.content,
    })
    .to_string();
    let occurrence = occurrence_by_base_key.entry(base_key.clone()).or_default();
    let key = format!("{base_key}#{occurrence}");
    *occurrence += 1;
    Some(key)
}

fn workflow_agent_wait_result(
    agent_id: Option<&str>,
    target: &str,
    event: ThreadPollEvent,
) -> Result<Value, WorkflowRuntimeError> {
    let (status, status_kind, text, content) = match &event {
        ThreadPollEvent::InterAgentCommunication { communication } => {
            workflow_agent_completion_fields(communication)
        }
        ThreadPollEvent::CommandExecutionNotification { .. } => {
            (None, "unknown", None, String::new())
        }
    };
    serde_json::to_value(serde_json::json!({
        "agentId": agent_id,
        "target": target,
        "status": status,
        "statusKind": status_kind,
        "text": text,
        "message": text,
        "content": content,
        "event": event,
        "events": [event],
        "sourceHint": "child_completion",
        "timedOut": false,
    }))
    .map_err(|err| WorkflowRuntimeError::invalid_request(err.to_string()))
}

fn workflow_agent_completion_fields(
    communication: &InterAgentCommunication,
) -> (Option<Value>, &'static str, Option<String>, String) {
    let content = communication.content.clone();
    let status = communication
        .status
        .as_ref()
        .and_then(|status| serde_json::to_value(status).ok());
    let status_kind = match communication.status.as_ref() {
        Some(AgentStatus::Completed(_)) => "completed",
        Some(AgentStatus::Errored(_)) => "errored",
        Some(AgentStatus::Shutdown) => "shutdown",
        Some(AgentStatus::NotFound) => "not_found",
        Some(AgentStatus::Interrupted) => "interrupted",
        Some(AgentStatus::PendingInit) => "pending_init",
        Some(AgentStatus::Running) => "running",
        None => "unknown",
    };
    let text = match communication.status.as_ref() {
        Some(AgentStatus::Completed(Some(text))) => Some(text.clone()),
        Some(AgentStatus::Completed(None)) => Some(content.clone()),
        Some(AgentStatus::Errored(message)) => Some(message.clone()),
        _ if !content.is_empty() => Some(content.clone()),
        _ => None,
    };
    (status, status_kind, text, content)
}

impl ThreadWorkflowRuntimeBridge {
    async fn cached_poll_result(&self) -> Option<ThreadPollEventResult> {
        let mut pending_events = self.pending_events.lock().await;
        if pending_events.is_empty() {
            return None;
        }
        let events: Vec<_> = pending_events
            .drain(..)
            .map(|cached| cached.event)
            .collect();
        Some(ThreadPollEventResult {
            timed_out: false,
            source_hint: Some("workflow_cached_event".to_string()),
            event: events.first().cloned(),
            events,
            waited_ms: 0,
            initial_timeout_ms: 0,
            current_timeout_ms: 0,
            hard_cap_timeout_ms: 0,
        })
    }

    async fn take_cached_matching_agent_completion(&self, target: &str) -> Option<ThreadPollEvent> {
        let mut pending_events = self.pending_events.lock().await;
        let mut consumed_completion_keys = self.consumed_completion_keys.lock().await;
        let mut index = 0;
        while index < pending_events.len() {
            let cached = pending_events.get(index)?;
            if cached
                .completion_key
                .as_ref()
                .is_some_and(|key| consumed_completion_keys.contains(key))
            {
                pending_events.remove(index);
                continue;
            }
            if is_target_agent_completion(&cached.event, target) {
                let cached = pending_events.remove(index)?;
                if let Some(key) = cached.completion_key.as_ref() {
                    consumed_completion_keys.insert(key.clone());
                }
                return Some(cached.event);
            }
            index += 1;
        }
        None
    }

    async fn push_pending_events(&self, events: Vec<CachedWorkflowEvent>) {
        if events.is_empty() {
            return;
        }
        let mut pending_events = self.pending_events.lock().await;
        pending_events.extend(events);
    }

    async fn split_wait_events(
        &self,
        events: Vec<ThreadPollEvent>,
        target: &str,
    ) -> (Option<ThreadPollEvent>, Vec<CachedWorkflowEvent>) {
        let mut consumed_completion_keys = self.consumed_completion_keys.lock().await;
        split_wait_events_for_bridge(events, target, &mut consumed_completion_keys)
    }
}

struct ThreadWorkflowProgressSink {
    thread_runtime: Weak<dyn WorkflowThreadRuntime>,
    turn: Option<Arc<dyn ThreadTurnCapability>>,
}

impl ThreadWorkflowProgressSink {
    fn new(
        thread_runtime: Weak<dyn WorkflowThreadRuntime>,
        turn: Option<Arc<dyn ThreadTurnCapability>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            thread_runtime,
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
            let Some(thread_runtime) = self.thread_runtime.upgrade() else {
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
            let _ = thread_runtime
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
    provider: Option<thread_service_api::ThreadSpawnAgentProvider>,
    agent_type: Option<String>,
    cwd: Option<codex_utils_absolute_path::AbsolutePathBuf>,
    model: Option<String>,
    reasoning_effort: Option<protocol::openai_models::ReasoningEffort>,
    service_tier: Option<String>,
    fork_turns: Option<String>,
    fork_context: Option<bool>,
}

impl WorkflowSpawnAgentArgs {
    fn into_request(self) -> Result<ThreadSpawnAgentRequest, FunctionCallError> {
        let fork_mode = self.fork_mode()?;
        Ok(ThreadSpawnAgentRequest {
            message: self.message,
            task_name: self.task_name,
            provider: self.provider,
            agent_type: self.agent_type,
            cwd: self.cwd,
            model: self.model,
            reasoning_effort: self.reasoning_effort,
            service_tier: self.service_tier,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn completion_event(author: &str, text: &str) -> ThreadPollEvent {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from(author).expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            text.to_string(),
            InterAgentOperation::ChildCompletion,
        )
        .with_status(AgentStatus::Completed(Some(text.to_string())));
        ThreadPollEvent::InterAgentCommunication { communication }
    }

    #[test]
    fn split_wait_events_returns_only_target_agent_completion() {
        let mut consumed = HashSet::new();
        let (matching, pending) = split_wait_events_for_bridge(
            vec![
                completion_event("/root/explorer", "research done"),
                completion_event("/root/owner", "implementation done"),
            ],
            "/root/owner",
            &mut consumed,
        );

        assert!(is_target_agent_completion(
            matching.as_ref().expect("target completion"),
            "/root/owner"
        ));
        assert_eq!(pending.len(), 1);
        assert!(is_target_agent_completion(
            &pending.first().expect("pending completion").event,
            "/root/explorer"
        ));
        assert_eq!(consumed.len(), 1);
    }

    #[test]
    fn split_wait_events_skips_consumed_target_completion_and_keeps_next_match() {
        let old_completion = completion_event("/root/owner", "same done");
        let mut occurrences = HashMap::new();
        let old_key =
            completion_event_key(&old_completion, &mut occurrences).expect("completion key");
        let mut consumed = HashSet::from([old_key]);
        let (matching, pending) = split_wait_events_for_bridge(
            vec![old_completion, completion_event("/root/owner", "same done")],
            "/root/owner",
            &mut consumed,
        );

        let matching = matching.expect("new target completion");
        let ThreadPollEvent::InterAgentCommunication { communication } = matching else {
            panic!("expected inter-agent communication");
        };
        assert_eq!(communication.content, "same done");
        assert!(pending.is_empty());
        assert_eq!(consumed.len(), 2);
    }

    #[test]
    fn split_wait_events_ignores_repeated_consumed_completion() {
        let old_completion = completion_event("/root/owner", "old done");
        let mut occurrences = HashMap::new();
        let old_key =
            completion_event_key(&old_completion, &mut occurrences).expect("completion key");
        let mut consumed = HashSet::from([old_key]);
        let (matching, pending) =
            split_wait_events_for_bridge(vec![old_completion], "/root/owner", &mut consumed);

        assert!(matching.is_none());
        assert!(pending.is_empty());
        assert_eq!(consumed.len(), 1);
    }

    #[test]
    fn split_wait_events_preserves_additional_target_completion_for_later_wait() {
        let mut consumed = HashSet::new();
        let (matching, pending) = split_wait_events_for_bridge(
            vec![
                completion_event("/root/owner", "first done"),
                completion_event("/root/owner", "second done"),
            ],
            "/root/owner",
            &mut consumed,
        );

        let matching = matching.expect("first target completion");
        let ThreadPollEvent::InterAgentCommunication { communication } = matching else {
            panic!("expected inter-agent communication");
        };
        assert_eq!(communication.content, "first done");
        assert_eq!(pending.len(), 1);
        assert!(is_target_agent_completion(
            &pending.first().expect("pending target completion").event,
            "/root/owner"
        ));
        assert_eq!(consumed.len(), 1);
    }

    #[test]
    fn workflow_agent_wait_result_exposes_stable_completion_fields() {
        let result = workflow_agent_wait_result(
            Some("owner"),
            "/root/owner",
            completion_event("/root/owner", "implementation done"),
        )
        .expect("wait result");

        assert_eq!(result["agentId"], "owner");
        assert_eq!(result["target"], "/root/owner");
        assert_eq!(result["statusKind"], "completed");
        assert_eq!(result["text"], "implementation done");
        assert_eq!(result["message"], "implementation done");
        assert_eq!(
            result["event"]["communication"]["author"],
            serde_json::json!("/root/owner")
        );
    }
}
