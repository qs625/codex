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

const TOOL_NAME: &str = "event_command_subscribe";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EventCommandSubscribeArgs {
    /// Shell command to run in the background. Each stdout line becomes an event.
    command: String,
    /// Optional working directory for the command.
    cwd: Option<String>,
    /// Optional short label included in the active monitor and emitted events.
    label: Option<String>,
}

#[derive(Serialize)]
struct EventCommandSubscribeResult {
    subscription_id: String,
    command: String,
    cwd: Option<String>,
    label: Option<String>,
}

pub(crate) struct EventCommandSubscribeTool {
    pub(crate) thread_id: ThreadId,
    pub(crate) registry: Arc<FsSubscriptionRegistry>,
}

#[async_trait]
impl ToolExecutor<ToolCall> for EventCommandSubscribeTool {
    type Output = ExtensionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(subscription_function_tool::<EventCommandSubscribeArgs>(
            TOOL_NAME,
            "Run a shell command in the background and inject an event whenever the command \
             writes a line to stdout. Use this for file monitors, long-running command monitors, \
             and command-exit notifications. The command is restarted automatically when the \
             thread resumes from persisted monitor metadata.\n\n\
             Parameters:\n\
             - `command`: shell command to run. Each stdout line is a separate event.\n\
             - `cwd`: optional working directory for the command. When omitted, the command \
             inherits the server process working directory.\n\
             - `label`: optional name included in active monitor lists and events.\n\n\
             Keep monitored commands quiet: redirect noisy output and only print lines that \
             should wake the model. Use `event_command_unsubscribe` to cancel a running monitor.",
        ))
    }

    async fn handle(&self, call: ToolCall) -> Result<Self::Output, FunctionCallError> {
        let args: EventCommandSubscribeArgs = parse_args(&call)?;
        if args.command.trim().is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "command must not be empty".to_string(),
            ));
        }
        let subscription_id = Uuid::now_v7().to_string();
        self.registry
            .subscribe_event_command(
                self.thread_id,
                args.command.clone(),
                args.cwd.clone(),
                args.label.clone(),
                subscription_id.clone(),
            )
            .await;
        Ok(JsonToolOutput::new(json!(EventCommandSubscribeResult {
            subscription_id,
            command: args.command,
            cwd: args.cwd,
            label: args.label,
        })))
    }
}
