use std::sync::Arc;

use async_trait::async_trait;
use codex_extension_api::ExtensionToolOutput;
use codex_extension_api::FunctionCallError;
use codex_extension_api::JsonToolOutput;
use codex_extension_api::ToolCall;
use codex_extension_api::ToolExecutor;
use codex_extension_api::ToolName;
use codex_extension_api::ToolSpec;
use codex_protocol::ThreadId;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::registry::FsSubscriptionRegistry;

use super::parse_args;
use super::subscription_function_tool;

const TOOL_NAME: &str = "schedule_unsubscribe";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScheduleUnsubscribeArgs {
    /// The subscription ID returned by `schedule_subscribe`.
    subscription_id: String,
}

pub(crate) struct ScheduleUnsubscribeTool {
    pub(crate) thread_id: ThreadId,
    pub(crate) registry: Arc<FsSubscriptionRegistry>,
}

#[async_trait]
impl ToolExecutor<ToolCall> for ScheduleUnsubscribeTool {
    type Output = ExtensionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(subscription_function_tool::<ScheduleUnsubscribeArgs>(
            TOOL_NAME,
            "Cancel a schedule subscription previously created with `schedule_subscribe`. \
             Use this when a recurring reminder is no longer needed, when the user cancels the \
             reminder, or when the watched workflow is complete and future schedule firings would \
             only create noise.",
        ))
    }

    async fn handle(&self, call: ToolCall) -> Result<Self::Output, FunctionCallError> {
        let args: ScheduleUnsubscribeArgs = parse_args(&call)?;
        let unsubscribed = self
            .registry
            .unsubscribe(self.thread_id, &args.subscription_id)
            .await;
        Ok(JsonToolOutput::new(json!({
            "unsubscribed": unsubscribed,
            "subscription_id": args.subscription_id,
        })))
    }
}
