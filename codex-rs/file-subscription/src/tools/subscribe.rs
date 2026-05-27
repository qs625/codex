use std::path::PathBuf;
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
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::registry::FsSubscriptionRegistry;

use super::parse_args;
use super::subscription_function_tool;

const TOOL_NAME: &str = "fs_subscribe";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FsSubscribeArgs {
    /// Absolute path to the file or directory to watch for changes.
    path: String,
    /// Whether to watch subdirectories recursively when `path` is a directory. Defaults to false.
    #[serde(default)]
    recursive: bool,
    /// Optional short label included in future change notifications so you can tell subscriptions apart.
    label: Option<String>,
}

#[derive(Serialize)]
struct FsSubscribeResult {
    subscription_id: String,
    path: String,
}

pub(crate) struct FsSubscribeTool {
    pub(crate) thread_id: ThreadId,
    pub(crate) registry: Arc<FsSubscriptionRegistry>,
}

#[async_trait]
impl ToolExecutor<ToolCall> for FsSubscribeTool {
    type Output = ExtensionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(subscription_function_tool::<FsSubscribeArgs>(
            TOOL_NAME,
            "Subscribe to file system changes at a path. \
             Use this when you need ongoing file or log change notifications instead of a \
             one-time read. The runtime debounces rapid file-system events and automatically \
             injects a notification into the conversation when the watched path changes.\n\n\
             Parameters:\n\
             - `path`: absolute file or directory path to watch.\n\
             - `recursive`: set to true when you need to watch a whole directory tree, not just \
             one path.\n\
             - `label`: optional name included in notifications so you can distinguish multiple \
             active watches.\n\n\
             Use this when:\n\
             - The user asks you to watch a generated file, report, artifact, or config and react \
             when it changes.\n\
             - You launch a long-running command and redirect stdout or stderr to a log file, \
             then want event-driven log monitoring instead of polling.\n\
             - You need to monitor a directory for newly written outputs such as test reports, \
             screenshots, or build artifacts.\n\n\
             Example requests:\n\
             - \"Watch `/tmp/build.log` and tell me if new errors appear.\"\n\
             - \"Keep an eye on `/workspace/out/report.json` and react when it is regenerated.\"\n\
             - \"Start the service with output redirected to `/tmp/server.log`, then monitor that \
             log file for readiness or crash messages.\"\n\n\
             Do not use this for a one-time file read; use ordinary file-reading tools for that. \
             When you only need to know that a process finished, prefer `process_exit_subscribe`.",
        ))
    }

    async fn handle(&self, call: ToolCall) -> Result<Self::Output, FunctionCallError> {
        let args: FsSubscribeArgs = parse_args(&call)?;
        let path = PathBuf::from(&args.path);
        let subscription_id = Uuid::now_v7().to_string();
        self.registry
            .subscribe_file(
                self.thread_id,
                path.clone(),
                args.recursive,
                args.label,
                subscription_id.clone(),
            )
            .await;
        Ok(JsonToolOutput::new(json!(FsSubscribeResult {
            subscription_id,
            path: path.display().to_string(),
        })))
    }
}
