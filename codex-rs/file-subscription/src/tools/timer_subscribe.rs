use std::sync::Arc;
use std::time::Duration;

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
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::registry::FsSubscriptionRegistry;

use super::parse_args;
use super::subscription_function_tool;

const TOOL_NAME: &str = "timer_subscribe";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TimerSubscribeArgs {
    /// Interval in milliseconds between timer wakeups.
    interval_ms: u64,
    /// Optional label used to identify this subscription in timer notifications.
    label: Option<String>,
}

#[derive(Serialize)]
struct TimerSubscribeResult {
    subscription_id: String,
    interval_ms: u64,
}

pub(crate) struct TimerSubscribeTool {
    pub(crate) thread_id: ThreadId,
    pub(crate) registry: Arc<FsSubscriptionRegistry>,
}

#[async_trait]
impl ToolExecutor<ToolCall> for TimerSubscribeTool {
    type Output = ExtensionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(subscription_function_tool::<TimerSubscribeArgs>(
            TOOL_NAME,
            "Subscribe to a repeating timer. A notification is automatically injected into the conversation every interval so you can observe and respond to the timer firing.",
        ))
    }

    async fn handle(&self, call: ToolCall) -> Result<Self::Output, FunctionCallError> {
        let args: TimerSubscribeArgs = parse_args(&call)?;
        if args.interval_ms == 0 {
            return Err(FunctionCallError::RespondToModel(
                "interval_ms must be greater than zero".to_string(),
            ));
        }
        let subscription_id = Uuid::now_v7().to_string();
        self.registry
            .subscribe_timer(
                self.thread_id,
                Duration::from_millis(args.interval_ms),
                args.label,
                subscription_id.clone(),
            )
            .await;
        Ok(JsonToolOutput::new(json!(TimerSubscribeResult {
            subscription_id,
            interval_ms: args.interval_ms,
        })))
    }
}
