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
    /// Absolute path to the file or directory to watch.
    path: String,
    /// Whether to watch subdirectories recursively. Defaults to false.
    #[serde(default)]
    recursive: bool,
    /// Optional label used to identify this subscription in change notifications.
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
             When the file or directory changes, a notification is automatically \
             injected into the conversation so you can observe and respond to the change.",
        ))
    }

    async fn handle(&self, call: ToolCall) -> Result<Self::Output, FunctionCallError> {
        let args: FsSubscribeArgs = parse_args(&call)?;
        let path = PathBuf::from(&args.path);
        let subscription_id = Uuid::now_v7().to_string();
        self.registry
            .subscribe(
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
