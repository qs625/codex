//! Built-in model tool handlers for persisted thread goals.
//!
//! Goal tools belong to the tool domain. The runtime owns argument parsing,
//! model-visible output, and tool specs; the embedding host owns persistence,
//! accounting, and thread events.

use codex_protocol::protocol::ThreadGoal;
use codex_protocol::protocol::ThreadGoalStatus;
use codex_thread_api::GoalApi;
use codex_thread_api::ThreadCapability;
use codex_tool_planning::CREATE_GOAL_TOOL_NAME;
use codex_tool_planning::GET_GOAL_TOOL_NAME;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::UPDATE_GOAL_TOOL_NAME;
use codex_tool_planning::create_create_goal_tool;
use codex_tool_planning::create_get_goal_tool;
use codex_tool_planning::create_update_goal_tool;
use codex_tool_runtime_api::ToolHandler;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolExecutor;
use codex_tool_types::ToolExecutorFuture;
use codex_tool_types::ToolPayload;
use serde::Deserialize;
use serde::Serialize;

use crate::FunctionToolOutput;
use codex_tool_runtime::ToolInvocation;

pub struct GetGoalHandler {
    service: std::sync::Arc<dyn GoalApi>,
}

impl GetGoalHandler {
    pub fn new(service: std::sync::Arc<dyn GoalApi>) -> Self {
        Self { service }
    }
}

pub struct CreateGoalHandler {
    service: std::sync::Arc<dyn GoalApi>,
}

impl CreateGoalHandler {
    pub fn new(service: std::sync::Arc<dyn GoalApi>) -> Self {
        Self { service }
    }
}

pub struct UpdateGoalHandler {
    service: std::sync::Arc<dyn GoalApi>,
}

impl UpdateGoalHandler {
    pub fn new(service: std::sync::Arc<dyn GoalApi>) -> Self {
        Self { service }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct CreateGoalArgs {
    objective: String,
    token_budget: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct UpdateGoalArgs {
    status: ThreadGoalStatus,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct GoalToolResponse {
    goal: Option<ThreadGoal>,
    remaining_tokens: Option<i64>,
    completion_budget_report: Option<String>,
}

#[derive(Clone, Copy)]
enum CompletionBudgetReport {
    Include,
    Omit,
}

impl GoalToolResponse {
    fn new(goal: Option<ThreadGoal>, report_mode: CompletionBudgetReport) -> Self {
        let remaining_tokens = goal.as_ref().and_then(|goal| {
            goal.token_budget
                .map(|budget| (budget - goal.tokens_used).max(0))
        });
        let completion_budget_report = match report_mode {
            CompletionBudgetReport::Include => goal
                .as_ref()
                .filter(|goal| goal.status == ThreadGoalStatus::Complete)
                .and_then(completion_budget_report),
            CompletionBudgetReport::Omit => None,
        };
        Self {
            goal,
            remaining_tokens,
            completion_budget_report,
        }
    }
}

impl<Session, Turn, Tracker> ToolExecutor<ToolInvocation<Session, Turn, Tracker>>
    for GetGoalHandler
where
    Turn: ThreadCapability,
    Session: Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(GET_GOAL_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_get_goal_tool())
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Session, Turn, Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            function_arguments(invocation.metadata.payload, "get_goal")?;
            let goal = self
                .service
                .get_thread_goal(&invocation.turn)
                .await
                .map_err(FunctionCallError::RespondToModel)?;
            goal_response(goal, CompletionBudgetReport::Omit)
        })
    }
}

impl<Session, Turn, Tracker, DiffContext> ToolHandler<ToolInvocation<Session, Turn, Tracker>, DiffContext>
    for GetGoalHandler
where
    Turn: ThreadCapability,
    Session: Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
    DiffContext: 'static,
{
}

impl<Session, Turn, Tracker> ToolExecutor<ToolInvocation<Session, Turn, Tracker>>
    for CreateGoalHandler
where
    Turn: ThreadCapability,
    Session: Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(CREATE_GOAL_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_create_goal_tool())
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Session, Turn, Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                turn,
                metadata,
                ..
            } = invocation;
            let arguments = function_arguments(metadata.payload, "goal")?;
            let args: CreateGoalArgs = parse_arguments(&arguments)?;
            let goal = self
                .service
                .create_thread_goal(&turn, args.objective, args.token_budget)
                .await
                .map_err(|err| {
                    if err.contains("already has a goal") {
                        FunctionCallError::RespondToModel(
                            "cannot create a new goal because this thread already has a goal; use update_goal only when the existing goal is complete"
                                .to_string(),
                        )
                    } else {
                        FunctionCallError::RespondToModel(err)
                    }
                })?;
            goal_response(Some(goal), CompletionBudgetReport::Omit)
        })
    }
}

impl<Session, Turn, Tracker, DiffContext> ToolHandler<ToolInvocation<Session, Turn, Tracker>, DiffContext>
    for CreateGoalHandler
where
    Turn: ThreadCapability,
    Session: Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
    DiffContext: 'static,
{
}

impl<Session, Turn, Tracker> ToolExecutor<ToolInvocation<Session, Turn, Tracker>>
    for UpdateGoalHandler
where
    Turn: ThreadCapability,
    Session: Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(UPDATE_GOAL_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_update_goal_tool())
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Session, Turn, Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                turn,
                metadata,
                ..
            } = invocation;
            let arguments = function_arguments(metadata.payload, "update_goal")?;

            let args: UpdateGoalArgs = parse_arguments(&arguments)?;
            if args.status != ThreadGoalStatus::Complete {
                return Err(FunctionCallError::RespondToModel(
                    "update_goal can only mark the existing goal complete; pause, resume, and budget-limited status changes are controlled by the user or system"
                        .to_string(),
                ));
            }
            let goal = self
                .service
                .complete_thread_goal(&turn)
                .await
                .map_err(FunctionCallError::RespondToModel)?;
            goal_response(Some(goal), CompletionBudgetReport::Include)
        })
    }
}

impl<Session, Turn, Tracker, DiffContext> ToolHandler<ToolInvocation<Session, Turn, Tracker>, DiffContext>
    for UpdateGoalHandler
where
    Turn: ThreadCapability,
    Session: Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
    DiffContext: 'static,
{
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
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })
}

fn goal_response(
    goal: Option<ThreadGoal>,
    completion_budget_report: CompletionBudgetReport,
) -> Result<FunctionToolOutput, FunctionCallError> {
    let response =
        serde_json::to_string_pretty(&GoalToolResponse::new(goal, completion_budget_report))
            .map_err(|err| FunctionCallError::Fatal(err.to_string()))?;
    Ok(FunctionToolOutput::from_text(response, Some(true)))
}

fn completion_budget_report(goal: &ThreadGoal) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(budget) = goal.token_budget {
        parts.push(format!("tokens used: {} of {budget}", goal.tokens_used));
    }
    if goal.time_used_seconds > 0 {
        parts.push(format!("time used: {} seconds", goal.time_used_seconds));
    }
    if parts.is_empty() {
        None
    } else {
        Some(format!(
            "Goal achieved. Report final budget usage to the user: {}.",
            parts.join("; ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::ThreadId;
    use pretty_assertions::assert_eq;

    #[test]
    fn completed_budgeted_goal_response_reports_final_usage() {
        let goal = ThreadGoal {
            thread_id: ThreadId::new(),
            objective: "Keep optimizing".to_string(),
            status: ThreadGoalStatus::Complete,
            token_budget: Some(10_000),
            tokens_used: 3_250,
            time_used_seconds: 75,
            created_at: 1,
            updated_at: 2,
        };

        let response = GoalToolResponse::new(Some(goal.clone()), CompletionBudgetReport::Include);

        assert_eq!(
            response,
            GoalToolResponse {
                goal: Some(goal),
                remaining_tokens: Some(6_750),
                completion_budget_report: Some(
                    "Goal achieved. Report final budget usage to the user: tokens used: 3250 of 10000; time used: 75 seconds."
                        .to_string()
                ),
            }
        );
    }

    #[test]
    fn completed_unbudgeted_goal_response_omits_budget_report() {
        let goal = ThreadGoal {
            thread_id: ThreadId::new(),
            objective: "Write a poem".to_string(),
            status: ThreadGoalStatus::Complete,
            token_budget: None,
            tokens_used: 120,
            time_used_seconds: 0,
            created_at: 1,
            updated_at: 2,
        };

        let response = GoalToolResponse::new(Some(goal.clone()), CompletionBudgetReport::Include);

        assert_eq!(
            response,
            GoalToolResponse {
                goal: Some(goal),
                remaining_tokens: None,
                completion_budget_report: None,
            }
        );
    }
}
