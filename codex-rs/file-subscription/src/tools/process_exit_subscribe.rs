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
    /// Session identifier returned by `exec_command` for the running process you want to watch.
    session_id: i32,
    /// Optional short label included in the exit notification so you can distinguish subscriptions.
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
            "Subscribe to a running `exec_command` session and inject a notification when that \
             process exits. The injected notification includes the exit status and the retained \
             aggregated process output captured for that session.\n\n\
             Parameters:\n\
             - `session_id`: the running `exec_command` session to watch.\n\
             - `label`: optional name included in the eventual exit notification.\n\n\
             Use this when:\n\
             - You only need a single completion event instead of continuous log updates.\n\
             - You are running build, test, lint, or batch commands where the most important \
             moment is process completion and you want the final retained error history when it \
             exits.\n\
             - You want a completion notification without having to keep checking the running \
             process manually.\n\n\
             Example requests:\n\
             - \"Run the test suite in the background and tell me when it finishes.\"\n\
             - \"Start the build and send me the final retained output once it exits.\"\n\
             - \"Watch this formatter run; I only care about the final failure summary if it \
             crashes.\"\n\n\
             When you need ongoing progress or log updates while the process is still running, \
             redirect output to a file and use `fs_subscribe` on that file instead.",
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
