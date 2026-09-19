use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

// Types for the TODO tool arguments matching codex-vscode/todo-mcp/src/main.rs
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    InProgress,
    Blocked,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PlanItemArg {
    pub step: String,
    pub status: StepStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct UpdatePlanArgs {
    /// Arguments for the `update_plan` todo/checklist tool (not plan mode).
    #[serde(default)]
    pub explanation: Option<String>,
    pub plan: Vec<PlanItemArg>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_plan_accepts_blocked_and_multiple_in_progress_steps() {
        let args: UpdatePlanArgs = serde_json::from_value(serde_json::json!({
            "explanation": "Parallel work with one blocker.",
            "plan": [
                { "step": "Build frontend", "status": "in_progress" },
                { "step": "Run backend validation", "status": "in_progress" },
                { "step": "Wait for credentials", "status": "blocked" },
                { "step": "Ship fix", "status": "pending" },
                { "step": "Write notes", "status": "completed" }
            ]
        }))
        .expect("plan args should accept blocked and multiple in_progress steps");

        assert!(matches!(args.plan[0].status, StepStatus::InProgress));
        assert!(matches!(args.plan[1].status, StepStatus::InProgress));
        assert!(matches!(args.plan[2].status, StepStatus::Blocked));
        assert!(matches!(args.plan[3].status, StepStatus::Pending));
        assert!(matches!(args.plan[4].status, StepStatus::Completed));
    }

    #[test]
    fn update_plan_keeps_legacy_three_state_compatibility() {
        let args: UpdatePlanArgs = serde_json::from_value(serde_json::json!({
            "plan": [
                { "step": "Start", "status": "pending" },
                { "step": "Work", "status": "in_progress" },
                { "step": "Done", "status": "completed" }
            ]
        }))
        .expect("legacy plan args should still deserialize");

        assert_eq!(args.plan.len(), 3);
    }
}
