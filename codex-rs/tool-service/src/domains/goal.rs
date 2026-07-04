use std::sync::Arc;

use crate::planning::CREATE_GOAL_TOOL_NAME;
use crate::planning::GET_GOAL_TOOL_NAME;
use crate::planning::ToolSpec;
use crate::planning::create_create_goal_tool;
use crate::planning::create_get_goal_tool;
use crate::planning::create_update_goal_tool;
use goal_service_api::GoalServiceApi;
use protocol::protocol::ThreadGoal;
use protocol::protocol::ThreadGoalStatus;
use serde::Deserialize;
use serde::Serialize;
use thread_service_api::ThreadSessionCapability;
use thread_service_api::ThreadTurnCapability;
use tool_service_api::AnyToolResult;
use tool_service_api::ErasedToolArgumentDiffConsumer;
use tool_service_api::FunctionCallError;
use tool_service_api::ToolCall;
use tool_service_api::ToolName;
use tool_service_api::UPDATE_GOAL_TOOL_NAME;

use crate::context::TypedToolSpecRequest;
use crate::output::FunctionToolOutput;

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
    goal_api: Arc<dyn GoalServiceApi>,
    session: &dyn ThreadSessionCapability,
    turn: &dyn ThreadTurnCapability,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    let result = match call.tool_name.name.as_str() {
        GET_GOAL_TOOL_NAME => goal_response(
            goal_api
                .get_thread_goal(session)
                .await
                .map_err(FunctionCallError::RespondToModel)?,
            CompletionBudgetReport::Omit,
        ),
        CREATE_GOAL_TOOL_NAME => {
            let args: CreateGoalArgs = parse_arguments(&call)?;
            let goal = goal_api
                .create_thread_goal(session, turn, args.objective, args.token_budget)
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
                .complete_thread_goal(session, turn)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use std::sync::Mutex;

    use protocol::ThreadId;
    use protocol::models::ResponseInputItem;
    use thread_service_api::SessionCapabilityFuture;
    use thread_service_api::ThreadCapability;
    use thread_service_api::ThreadSessionCapability;
    use tool_service_api::ToolOutput;

    struct MockTurn;
    struct MockSession;

    impl ThreadCapability for MockTurn {
        fn as_any(&self) -> &(dyn Any + Send + Sync) {
            self
        }
    }

    impl ThreadTurnCapability for MockTurn {}

    impl ThreadSessionCapability for MockSession {
        fn as_any(&self) -> &(dyn Any + Send + Sync) {
            self
        }

        fn into_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
            self
        }

        fn conversation_id(&self) -> ThreadId {
            ThreadId::new()
        }

        fn require_persisted_state_db<'a>(
            &'a self,
        ) -> SessionCapabilityFuture<'a, Result<state_api::SharedStateDbRuntime, String>> {
            Box::pin(async { unreachable!("mock goal api should handle state db access") })
        }
    }

    struct MockGoalApi {
        goal: Mutex<Option<ThreadGoal>>,
    }

    impl MockGoalApi {
        fn with_goal(goal: Option<ThreadGoal>) -> Self {
            Self {
                goal: Mutex::new(goal),
            }
        }
    }

    impl GoalServiceApi for MockGoalApi {
        fn get_thread_goal<'a>(
            &'a self,
            _session: &'a dyn ThreadSessionCapability,
        ) -> SessionCapabilityFuture<'a, Result<Option<ThreadGoal>, String>> {
            Box::pin(async move { Ok(self.goal.lock().expect("goal lock").clone()) })
        }

        fn create_thread_goal<'a>(
            &'a self,
            _session: &'a dyn ThreadSessionCapability,
            _turn: &'a dyn ThreadTurnCapability,
            objective: String,
            token_budget: Option<i64>,
        ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
            Box::pin(async move {
                let mut goal = self.goal.lock().expect("goal lock");
                if goal.is_some() {
                    return Err("thread already has a goal".to_string());
                }
                let created = ThreadGoal {
                    thread_id: ThreadId::new(),
                    objective,
                    status: ThreadGoalStatus::Active,
                    token_budget,
                    tokens_used: 0,
                    time_used_seconds: 0,
                    created_at: 1,
                    updated_at: 1,
                };
                *goal = Some(created.clone());
                Ok(created)
            })
        }

        fn complete_thread_goal<'a>(
            &'a self,
            _session: &'a dyn ThreadSessionCapability,
            _turn: &'a dyn ThreadTurnCapability,
        ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
            Box::pin(async move {
                let mut goal = self.goal.lock().expect("goal lock");
                let Some(existing) = goal.as_mut() else {
                    return Err("goal not found".to_string());
                };
                existing.status = ThreadGoalStatus::Complete;
                existing.tokens_used = 77;
                existing.time_used_seconds = 12;
                existing.updated_at = 2;
                Ok(existing.clone())
            })
        }
    }

    fn function_call(tool_name: &str, call_id: &str, arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            call_id: call_id.to_string(),
            tool_name: ToolName::plain(tool_name),
            payload: tool_service_api::ToolPayload::Function {
                arguments: arguments.to_string(),
            },
        }
    }

    fn output_text(result: &AnyToolResult) -> String {
        match result
            .result
            .to_response_item(&result.call_id, &result.payload)
        {
            ResponseInputItem::FunctionCallOutput { output, .. }
            | ResponseInputItem::CustomToolCallOutput { output, .. } => {
                output.body.to_text().unwrap_or_default()
            }
            other => panic!("unexpected tool response item: {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_goal_tool_rejects_existing_goal() {
        let existing = ThreadGoal {
            thread_id: ThreadId::new(),
            objective: "Keep the watcher alive".to_string(),
            status: ThreadGoalStatus::Active,
            token_budget: Some(123),
            tokens_used: 0,
            time_used_seconds: 0,
            created_at: 1,
            updated_at: 1,
        };
        let goal_api = Arc::new(MockGoalApi::with_goal(Some(existing.clone())));
        let session = MockSession;
        let turn = MockTurn;
        let response = dispatch(
            goal_api,
            &session,
            &turn,
            function_call(
                CREATE_GOAL_TOOL_NAME,
                "create-goal-2",
                serde_json::json!({
                    "objective": "Replace the watcher",
                    "token_budget": 456,
                }),
            ),
        )
        .await;

        let Err(FunctionCallError::RespondToModel(output)) = response else {
            panic!("expected create_goal to reject an existing goal");
        };
        assert_eq!(
            output,
            "cannot create a new goal because this thread already has a goal; use update_goal only when the existing goal is complete"
        );
    }

    #[tokio::test]
    async fn update_goal_tool_rejects_pausing_goal() {
        let goal_api = Arc::new(MockGoalApi::with_goal(None));
        let session = MockSession;
        let turn = MockTurn;
        let response = dispatch(
            goal_api,
            &session,
            &turn,
            function_call(
                UPDATE_GOAL_TOOL_NAME,
                "pause-goal",
                serde_json::json!({
                    "status": "paused",
                }),
            ),
        )
        .await;

        let Err(FunctionCallError::RespondToModel(output)) = response else {
            panic!("expected update_goal to reject pausing a goal");
        };
        assert_eq!(
            output,
            "update_goal can only mark the existing goal complete; pause, resume, and budget-limited status changes are controlled by the user or system"
        );
    }

    #[tokio::test]
    async fn update_goal_tool_marks_goal_complete_and_reports_budget_usage() {
        let goal_api = Arc::new(MockGoalApi::with_goal(Some(ThreadGoal {
            thread_id: ThreadId::new(),
            objective: "Keep the watcher alive".to_string(),
            status: ThreadGoalStatus::Active,
            token_budget: Some(123),
            tokens_used: 0,
            time_used_seconds: 0,
            created_at: 1,
            updated_at: 1,
        })));
        let session = MockSession;
        let turn = MockTurn;
        let result = dispatch(
            goal_api,
            &session,
            &turn,
            function_call(
                UPDATE_GOAL_TOOL_NAME,
                "complete-goal",
                serde_json::json!({
                    "status": "complete",
                }),
            ),
        )
        .await
        .expect("update_goal should mark the goal complete");

        let output: serde_json::Value =
            serde_json::from_str(&output_text(&result)).expect("goal tool json");
        assert_eq!(output["goal"]["status"], "complete");
        assert_eq!(output["remainingTokens"], 46);
        assert_eq!(
            output["completionBudgetReport"],
            "Goal achieved. Report final budget usage to the user: tokens used: 77 of 123; time used: 12 seconds."
        );
    }
}
