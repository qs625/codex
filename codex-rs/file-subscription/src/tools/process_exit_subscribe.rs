use std::sync::Arc;

use async_trait::async_trait;
use codex_core::UnifiedExecProcessManager;
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

const TOOL_NAME: &str = "process_exit_subscribe";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProcessExitSubscribeArgs {
    /// Session identifier returned by `exec_command` for a running process.
    session_id: i32,
    /// Optional label used to identify this subscription in exit notifications.
    label: Option<String>,
}

#[derive(Serialize)]
struct ProcessExitSubscribeResult {
    session_id: i32,
    subscription_id: String,
}

pub(crate) struct ProcessExitSubscribeTool {
    pub(crate) thread_id: ThreadId,
    pub(crate) registry: Arc<FsSubscriptionRegistry>,
    pub(crate) unified_exec_manager: Arc<UnifiedExecProcessManager>,
}

#[async_trait]
impl ToolExecutor<ToolCall> for ProcessExitSubscribeTool {
    type Output = ExtensionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(subscription_function_tool::<ProcessExitSubscribeArgs>(
            TOOL_NAME,
            "Subscribe to a running exec_command session and inject a notification when that process exits.",
        ))
    }

    async fn handle(&self, call: ToolCall) -> Result<Self::Output, FunctionCallError> {
        let args: ProcessExitSubscribeArgs = parse_args(&call)?;
        let subscription_id = Uuid::now_v7().to_string();
        let subscribed = self
            .registry
            .subscribe_process_exit(
                self.thread_id,
                args.session_id,
                args.label,
                subscription_id.clone(),
                Arc::clone(&self.unified_exec_manager),
            )
            .await;
        if !subscribed {
            return Err(FunctionCallError::RespondToModel(format!(
                "unknown process session_id: {}",
                args.session_id
            )));
        }
        Ok(JsonToolOutput::new(json!(ProcessExitSubscribeResult {
            session_id: args.session_id,
            subscription_id,
        })))
    }
}
