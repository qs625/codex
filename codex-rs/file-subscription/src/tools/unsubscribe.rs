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

const TOOL_NAME: &str = "fs_unsubscribe";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FsUnsubscribeArgs {
    /// The subscription ID returned by `fs_subscribe`.
    subscription_id: String,
}

pub(crate) struct FsUnsubscribeTool {
    pub(crate) thread_id: ThreadId,
    pub(crate) registry: Arc<FsSubscriptionRegistry>,
}

#[async_trait]
impl ToolExecutor<ToolCall> for FsUnsubscribeTool {
    type Output = ExtensionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(subscription_function_tool::<FsUnsubscribeArgs>(
            TOOL_NAME,
            "Cancel a file subscription previously created with fs_subscribe. \
             Use this when you no longer need to watch the file or directory, such as after the \
             user-requested monitoring task is finished, after the relevant process has exited, or \
             after you have already seen the file change you were waiting for. \
             Unsubscribe when the watch is no longer useful so future file changes do not keep \
             injecting notifications into the conversation.",
        ))
    }

    async fn handle(&self, call: ToolCall) -> Result<Self::Output, FunctionCallError> {
        let args: FsUnsubscribeArgs = parse_args(&call)?;
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
