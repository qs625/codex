use std::sync::Arc;

use codex_extension_api::ExtensionToolOutput;
use codex_extension_api::FunctionCallError;
use codex_extension_api::JsonToolOutput;
use codex_extension_api::ToolCall;
use codex_extension_api::ToolExecutor;
use codex_extension_api::ToolName;
use codex_extension_api::ToolSpec;
use codex_protocol::ThreadId;
use codex_protocol::subscriptions::ScheduleSpec;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::registry::FsSubscriptionRegistry;

use super::parse_args;
use super::schedule::CompiledSchedule;
use super::subscription_function_tool;

const TOOL_NAME: &str = "schedule_subscribe";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScheduleSubscribeArgs {
    /// Structured schedule rule describing when the reminder should fire.
    schedule: ScheduleSpec,
    /// Optional short label included in future schedule notifications.
    label: Option<String>,
}

#[derive(Serialize)]
struct ScheduleSubscribeResult {
    subscription_id: String,
    next_fire_at: String,
    schedule_summary: String,
}

pub(crate) struct ScheduleSubscribeTool {
    pub(crate) thread_id: ThreadId,
    pub(crate) registry: Arc<FsSubscriptionRegistry>,
}

impl ToolExecutor<ToolCall> for ScheduleSubscribeTool {
    type Output = ExtensionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(subscription_function_tool::<ScheduleSubscribeArgs>(
            TOOL_NAME,
            "Subscribe to a structured schedule and inject a notification when it fires. \
             Use this for one-time reminders, recurring fixed intervals, daily reminders, or \
             weekly reminders on specific weekdays.\n\n\
             Parameters:\n\
             - `schedule`: the scheduling rule. Use `once_after` for a one-time relative delay, \
             `once_at` for a one-time absolute timestamp, `every_interval` for repeating fixed \
             intervals, `every_day_at` for a daily wall-clock reminder, or `every_week_at` for \
             weekly reminders on one or more weekdays.\n\
             - `label`: optional name included in future notifications so you can distinguish \
             multiple active schedules.\n\n\
             Use this when:\n\
             - The user asks for a reminder in a fixed amount of time, such as \"remind me in \
             two minutes\".\n\
             - The user asks for a one-time reminder at a specific future timestamp.\n\
             - The user asks for a repeating daily or weekly reminder at a wall-clock time.\n\
             - The user asks to poll or revisit something on a fixed interval.\n\n\
             Example requests:\n\
             - \"Remind me once in 2 minutes.\"\n\
             - \"Remind me next Wednesday at 3pm.\"\n\
             - \"Check this every 5 minutes.\"\n\
             - \"Trigger this every Tuesday at 09:00 Asia/Shanghai.\"\n\n\
             Use `schedule_unsubscribe` when a recurring schedule is no longer needed. \
             One-shot schedules end automatically after they fire once.",
        ))
    }

    fn handle<'a>(
        &'a self,
        call: ToolCall,
    ) -> codex_extension_api::ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let args: ScheduleSubscribeArgs = parse_args(&call)?;
            let compiled = CompiledSchedule::compile(args.schedule.clone())
                .map_err(FunctionCallError::RespondToModel)?;
            let next_fire_at = compiled
                .next_fire_at(chrono::Utc::now())
                .map_err(FunctionCallError::RespondToModel)?;
            let schedule_summary = compiled.summary();
            let subscription_id = Uuid::now_v7().to_string();
            self.registry
                .subscribe_schedule(
                    self.thread_id,
                    args.schedule,
                    compiled,
                    args.label,
                    subscription_id.clone(),
                )
                .await;
            Ok(JsonToolOutput::new(json!(ScheduleSubscribeResult {
                subscription_id,
                next_fire_at: next_fire_at.to_rfc3339(),
                schedule_summary,
            })))
        })
    }
}
