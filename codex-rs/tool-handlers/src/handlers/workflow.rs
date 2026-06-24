use crate::FunctionToolOutput;
use codex_protocol::models::WorkflowRunProgressKind;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::WORKFLOW_ABORT_TOOL_NAME;
use codex_tool_planning::WORKFLOW_DESCRIBE_TOOL_NAME;
use codex_tool_planning::WORKFLOW_LIST_TOOL_NAME;
use codex_tool_planning::WORKFLOW_RESUME_TOOL_NAME;
use codex_tool_planning::WORKFLOW_START_TOOL_NAME;
use codex_tool_planning::WORKFLOW_STATUS_TOOL_NAME;
use codex_tool_planning::create_workflow_abort_tool;
use codex_tool_planning::create_workflow_describe_tool;
use codex_tool_planning::create_workflow_list_tool;
use codex_tool_planning::create_workflow_resume_tool;
use codex_tool_planning::create_workflow_start_tool;
use codex_tool_planning::create_workflow_status_tool;
use codex_tool_runtime::ToolInvocation;
use codex_tool_runtime_api::ToolHandler;
use codex_tool_runtime_api::WorkflowToolHost;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolExecutor;
use codex_tool_types::ToolExecutorFuture;
use codex_tool_types::ToolPayload;
use codex_workflow_api::WorkflowAbortArgs;
use codex_workflow_api::WorkflowDescribeArgs;
use codex_workflow_api::WorkflowResumeArgs;
use codex_workflow_api::WorkflowRunStatus;
use codex_workflow_api::WorkflowRunUpdateError;
use codex_workflow_api::WorkflowRunUpdateReceiver;
use codex_workflow_api::WorkflowStartArgs;
use codex_workflow_api::WorkflowStatusArgs;
use codex_workflow_api::workflow_tool_output_json;
use serde::de::DeserializeOwned;

pub struct WorkflowListHandler<Host> {
    host: Host,
}

pub struct WorkflowDescribeHandler<Host> {
    host: Host,
}

pub struct WorkflowStartHandler<Host> {
    host: Host,
}

pub struct WorkflowStatusHandler<Host> {
    host: Host,
}

pub struct WorkflowResumeHandler<Host> {
    host: Host,
}

pub struct WorkflowAbortHandler<Host> {
    host: Host,
}

impl<Host> WorkflowListHandler<Host> {
    pub fn new(host: Host) -> Self {
        Self { host }
    }
}

impl<Host> WorkflowDescribeHandler<Host> {
    pub fn new(host: Host) -> Self {
        Self { host }
    }
}

impl<Host> WorkflowStartHandler<Host> {
    pub fn new(host: Host) -> Self {
        Self { host }
    }
}

impl<Host> WorkflowStatusHandler<Host> {
    pub fn new(host: Host) -> Self {
        Self { host }
    }
}

impl<Host> WorkflowResumeHandler<Host> {
    pub fn new(host: Host) -> Self {
        Self { host }
    }
}

impl<Host> WorkflowAbortHandler<Host> {
    pub fn new(host: Host) -> Self {
        Self { host }
    }
}

impl<Host> ToolExecutor<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>>
    for WorkflowListHandler<Host>
where
    Host: WorkflowToolHost,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(WORKFLOW_LIST_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_workflow_list_tool())
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation { turn, metadata, .. } = invocation;
            match metadata.payload {
                ToolPayload::Function { .. } => {
                    let registry = self.host.load_workflow_registry(&turn);
                    json_output(&registry)
                }
                _ => Err(FunctionCallError::RespondToModel(
                    "workflow_list handler received unsupported payload".to_string(),
                )),
            }
        })
    }
}

impl<Host> ToolHandler<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, Host::DiffContext>
    for WorkflowListHandler<Host>
where
    Host: WorkflowToolHost,
{
}

impl<Host> ToolExecutor<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>>
    for WorkflowDescribeHandler<Host>
where
    Host: WorkflowToolHost,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(WORKFLOW_DESCRIBE_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_workflow_describe_tool())
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation { turn, metadata, .. } = invocation;
            let arguments = function_arguments(metadata.payload, WORKFLOW_DESCRIBE_TOOL_NAME)?;
            let args: WorkflowDescribeArgs = parse_arguments(&arguments)?;
            let workflow = args.workflow().map_err(FunctionCallError::RespondToModel)?;

            let registry = self.host.load_workflow_registry(&turn);
            let details = registry
                .details(workflow)
                .map_err(FunctionCallError::RespondToModel)?;
            json_output(&details)
        })
    }
}

impl<Host> ToolHandler<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, Host::DiffContext>
    for WorkflowDescribeHandler<Host>
where
    Host: WorkflowToolHost,
{
}

impl<Host> ToolExecutor<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>>
    for WorkflowStartHandler<Host>
where
    Host: WorkflowToolHost,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(WORKFLOW_START_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_workflow_start_tool())
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                cancellation_token,
                tracker,
                metadata,
            } = invocation;
            let arguments = function_arguments(metadata.payload, WORKFLOW_START_TOOL_NAME)?;
            let args: WorkflowStartArgs = parse_arguments(&arguments)?;
            let workflow = args
                .workflow()
                .map(str::to_string)
                .map_err(FunctionCallError::RespondToModel)?;

            let registry = self.host.load_workflow_registry(&turn);
            let controller = self.host.workflow_run_controller(&session);
            let updates = controller.subscribe();
            let bridge = self.host.create_workflow_runtime_bridge(
                session.clone(),
                turn.clone(),
                cancellation_token.clone(),
                tracker.clone(),
            );
            let run = controller
                .start_with_bridge(
                    &registry,
                    &workflow,
                    args.inputs.unwrap_or_default(),
                    bridge,
                )
                .await
                .map_err(FunctionCallError::RespondToModel)?;
            self.host
                .record_workflow_progress(&session, &turn, &run, WorkflowRunProgressKind::Started)
                .await;
            record_terminal_workflow_progress(
                self.host.clone(),
                session,
                turn,
                updates,
                run.run_id.clone(),
            );
            json_output(&run)
        })
    }
}

impl<Host> ToolHandler<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, Host::DiffContext>
    for WorkflowStartHandler<Host>
where
    Host: WorkflowToolHost,
{
}

impl<Host> ToolExecutor<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>>
    for WorkflowStatusHandler<Host>
where
    Host: WorkflowToolHost,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(WORKFLOW_STATUS_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_workflow_status_tool())
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session, metadata, ..
            } = invocation;
            let arguments = function_arguments(metadata.payload, WORKFLOW_STATUS_TOOL_NAME)?;
            let args: WorkflowStatusArgs = parse_arguments(&arguments)?;
            let run_id = args.run_id().map_err(FunctionCallError::RespondToModel)?;

            let run = self
                .host
                .workflow_run_controller(&session)
                .status(run_id)
                .await
                .map_err(FunctionCallError::RespondToModel)?;
            json_output(&run)
        })
    }
}

impl<Host> ToolHandler<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, Host::DiffContext>
    for WorkflowStatusHandler<Host>
where
    Host: WorkflowToolHost,
{
}

impl<Host> ToolExecutor<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>>
    for WorkflowResumeHandler<Host>
where
    Host: WorkflowToolHost,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(WORKFLOW_RESUME_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_workflow_resume_tool())
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                cancellation_token,
                tracker,
                metadata,
            } = invocation;
            let arguments = function_arguments(metadata.payload, WORKFLOW_RESUME_TOOL_NAME)?;
            let args: WorkflowResumeArgs = parse_arguments(&arguments)?;
            let run_id = args
                .run_id()
                .map(str::to_string)
                .map_err(FunctionCallError::RespondToModel)?;

            let controller = self.host.workflow_run_controller(&session);
            let updates = controller.subscribe();
            let bridge = self.host.create_workflow_runtime_bridge(
                session.clone(),
                turn.clone(),
                cancellation_token.clone(),
                tracker.clone(),
            );
            let run = controller
                .resume_with_bridge(&run_id, args.inputs, bridge)
                .await
                .map_err(FunctionCallError::RespondToModel)?;
            self.host
                .record_workflow_progress(&session, &turn, &run, WorkflowRunProgressKind::Resumed)
                .await;
            record_terminal_workflow_progress(
                self.host.clone(),
                session,
                turn,
                updates,
                run.run_id.clone(),
            );
            json_output(&run)
        })
    }
}

impl<Host> ToolHandler<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, Host::DiffContext>
    for WorkflowResumeHandler<Host>
where
    Host: WorkflowToolHost,
{
}

impl<Host> ToolExecutor<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>>
    for WorkflowAbortHandler<Host>
where
    Host: WorkflowToolHost,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(WORKFLOW_ABORT_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_workflow_abort_tool())
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                metadata,
                ..
            } = invocation;
            let arguments = function_arguments(metadata.payload, WORKFLOW_ABORT_TOOL_NAME)?;
            let args: WorkflowAbortArgs = parse_arguments(&arguments)?;
            let run_id = args
                .run_id()
                .map(str::to_string)
                .map_err(FunctionCallError::RespondToModel)?;

            let run = self
                .host
                .workflow_run_controller(&session)
                .abort(&run_id, args.reason)
                .await
                .map_err(FunctionCallError::RespondToModel)?;
            self.host
                .record_workflow_progress(&session, &turn, &run, WorkflowRunProgressKind::Aborted)
                .await;
            json_output(&run)
        })
    }
}

impl<Host> ToolHandler<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, Host::DiffContext>
    for WorkflowAbortHandler<Host>
where
    Host: WorkflowToolHost,
{
}

fn json_output<T: serde::Serialize>(value: &T) -> Result<FunctionToolOutput, FunctionCallError> {
    workflow_tool_output_json(value)
        .map(|text| FunctionToolOutput::from_text(text, Some(true)))
        .map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize workflow tool output: {err}"))
        })
}

fn function_arguments(payload: ToolPayload, tool_name: &str) -> Result<String, FunctionCallError> {
    match payload {
        ToolPayload::Function { arguments } => Ok(arguments),
        _ => Err(FunctionCallError::RespondToModel(format!(
            "{tool_name} handler received unsupported payload"
        ))),
    }
}

fn parse_arguments<T>(arguments: &str) -> Result<T, FunctionCallError>
where
    T: DeserializeOwned,
{
    serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })
}

fn record_terminal_workflow_progress<Host>(
    host: Host,
    session: Host::Session,
    turn: Host::Turn,
    mut updates: Box<dyn WorkflowRunUpdateReceiver>,
    run_id: String,
) where
    Host: WorkflowToolHost,
{
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
                host.record_workflow_progress(&session, &turn, &run, kind)
                    .await;
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
