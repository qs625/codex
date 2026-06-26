use std::sync::Arc;

use codex_protocol::protocol::ThreadGoal;
use codex_protocol::protocol::ThreadGoalStatus;
use codex_thread_api::GoalApi;
use codex_thread_api::ThreadCapability;
use codex_tool_runtime::FunctionToolOutput;
use codex_tool_runtime_api::AnyToolResult;
use codex_tool_service_api::ErasedToolArgumentDiffConsumer;
use codex_tool_planning::CREATE_GOAL_TOOL_NAME;
use codex_tool_planning::GET_GOAL_TOOL_NAME;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::UPDATE_GOAL_TOOL_NAME;
use codex_tool_planning::create_create_goal_tool;
use codex_tool_planning::create_get_goal_tool;
use codex_tool_planning::create_update_goal_tool;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCall;
use codex_tool_types::ToolName;
use serde::Deserialize;
use serde::Serialize;

use crate::context::TypedToolSpecRequest;

pub(crate) fn specs(_request: &TypedToolSpecRequest<'_>) -> Vec<ToolSpec> {
    vec![
        create_get_goal_tool(),
        create_create_goal_tool(),
        create_update_goal_tool(),
    ]
}

pub(crate) fn owns_tool_name(_request: &TypedToolSpecRequest<'_>, tool_name: &ToolName) -> bool {
    tool_name.namespace.is_none()
        && matches!(
            tool_name.name.as_str(),
            GET_GOAL_TOOL_NAME | CREATE_GOAL_TOOL_NAME | UPDATE_GOAL_TOOL_NAME
        )
}

pub(crate) fn create_diff_consumer(
    _request: &TypedToolSpecRequest<'_>,
    _tool_name: &ToolName,
) -> Option<Box<dyn ErasedToolArgumentDiffConsumer>> {
    None
}

pub(crate) fn supports_parallel(_request: &TypedToolSpecRequest<'_>, _call: &ToolCall) -> bool {
    false
}

pub(crate) async fn dispatch(
    goal_api: Arc<dyn GoalApi>,
    turn: &dyn ThreadCapability,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    let result = match call.tool_name.name.as_str() {
        GET_GOAL_TOOL_NAME => goal_response(
            goal_api
                .get_thread_goal(turn)
                .await
                .map_err(FunctionCallError::RespondToModel)?,
            CompletionBudgetReport::Omit,
        ),
        CREATE_GOAL_TOOL_NAME => {
            let args: CreateGoalArgs = parse_arguments(&call)?;
            let goal = goal_api
                .create_thread_goal(turn, args.objective, args.token_budget)
                .await
                .map_err(map_create_goal_error)?;
            goal_response(Some(goal), CompletionBudgetReport::Omit)
        }
        UPDATE_GOAL_TOOL_NAME => {
            let args: UpdateGoalArgs = parse_arguments(&call)?;
            if args.status != ThreadGoalStatus::Complete {
                return Err(FunctionCallError::RespondToModel(
                    "update_goal can only mark the existing goal complete; pause, resume, and budget-limited status changes are controlled by the user or system"
                        .to_string(),
                ));
            }
            let goal = goal_api
                .complete_thread_goal(turn)
                .await
                .map_err(FunctionCallError::RespondToModel)?;
            goal_response(Some(goal), CompletionBudgetReport::Include)
        }
        _ => Err(FunctionCallError::Fatal(format!(
            "unsupported goal tool {}",
            call.tool_name
        ))),
    }?;

    Ok(AnyToolResult {
        call_id: call.call_id,
        payload: call.payload,
        result: Box::new(result),
        post_tool_use_payload: None,
    })
}

pub(crate) fn tool_output_for_state(
    tool_name: &ToolName,
    goal: Option<ThreadGoal>,
) -> Result<FunctionToolOutput, FunctionCallError> {
    let report_mode = match tool_name.name.as_str() {
        UPDATE_GOAL_TOOL_NAME => CompletionBudgetReport::Include,
        GET_GOAL_TOOL_NAME | CREATE_GOAL_TOOL_NAME => CompletionBudgetReport::Omit,
        _ => {
            return Err(FunctionCallError::Fatal(format!(
                "unsupported goal tool {}",
                tool_name
            )));
        }
    };
    goal_response(goal, report_mode)
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

fn parse_arguments<T>(call: &ToolCall) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(call.function_arguments()?).map_err(|err| {
        FunctionCallError::RespondToModel(format!(
            "failed to parse {} arguments: {err}",
            call.tool_name
        ))
    })
}

fn map_create_goal_error(err: String) -> FunctionCallError {
    if err.contains("already has a goal") {
        FunctionCallError::RespondToModel(
            "cannot create a new goal because this thread already has a goal; use update_goal only when the existing goal is complete"
                .to_string(),
        )
    } else {
        FunctionCallError::RespondToModel(err)
    }
}

fn goal_response(
    goal: Option<ThreadGoal>,
    report_mode: CompletionBudgetReport,
) -> Result<FunctionToolOutput, FunctionCallError> {
    serde_json::to_string_pretty(&GoalToolResponse::new(goal, report_mode))
        .map(|text| FunctionToolOutput::from_text(text, Some(true)))
        .map_err(|err| FunctionCallError::Fatal(err.to_string()))
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
