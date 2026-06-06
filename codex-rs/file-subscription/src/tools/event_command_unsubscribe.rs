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

const TOOL_NAME: &str = "event_command_unsubscribe";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EventCommandUnsubscribeArgs {
    /// The subscription ID returned by `event_command_subscribe`.
    subscription_id: String,
}

pub(crate) struct EventCommandUnsubscribeTool {
    pub(crate) thread_id: ThreadId,
    pub(crate) registry: Arc<FsSubscriptionRegistry>,
}

#[async_trait]
impl ToolExecutor<ToolCall> for EventCommandUnsubscribeTool {
    type Output = ExtensionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(subscription_function_tool::<EventCommandUnsubscribeArgs>(
            TOOL_NAME,
            "Cancel an EventCommand monitor previously created with `event_command_subscribe`. \
             Cancelling stops the background command when possible and removes the active monitor.",
        ))
    }

    async fn handle(&self, call: ToolCall) -> Result<Self::Output, FunctionCallError> {
        let args: EventCommandUnsubscribeArgs = parse_args(&call)?;
        let cancelled = self
            .registry
            .unsubscribe(self.thread_id, &args.subscription_id)
            .await;
        Ok(JsonToolOutput::new(json!({
            "cancelled": cancelled,
            "subscription_id": args.subscription_id,
        })))
    }
}
