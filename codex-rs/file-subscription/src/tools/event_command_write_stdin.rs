use std::sync::Arc;

use codex_extension_api::ExtensionToolOutput;
use codex_extension_api::FunctionCallError;
use codex_extension_api::JsonToolOutput;
use codex_extension_api::ToolCall;
use codex_extension_api::ToolExecutor;
use codex_extension_api::ToolName;
use codex_extension_api::ToolSpec;
use protocol::ThreadId;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

use crate::registry::FsSubscriptionRegistry;

use super::parse_args;
use super::subscription_function_tool;

const TOOL_NAME: &str = "event_command_write_stdin";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EventCommandWriteStdinArgs {
    /// The subscription ID returned by `event_command_subscribe`.
    subscription_id: String,
    /// Non-empty bytes to write to the event command's stdin.
    chars: String,
}

#[derive(Serialize)]
struct EventCommandWriteStdinResult {
    subscription_id: String,
    bytes_written: usize,
}

pub(crate) struct EventCommandWriteStdinTool {
    pub(crate) thread_id: ThreadId,
    pub(crate) registry: Arc<FsSubscriptionRegistry>,
}

impl ToolExecutor<ToolCall> for EventCommandWriteStdinTool {
    type Output = ExtensionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(subscription_function_tool::<EventCommandWriteStdinArgs>(
            TOOL_NAME,
            "Write non-empty stdin to a running EventCommand monitor by the stable \
             `subscription_id` returned from `event_command_subscribe`. Use this only to send \
             real input to the monitored process; EventCommand output is delivered as thread \
             events, and command completion/log watching should stay on the subscription path.",
        ))
    }

    fn handle<'a>(
        &'a self, call: ToolCall,
    ) -> codex_extension_api::ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
                    let args: EventCommandWriteStdinArgs = parse_args(&call)?;
                    if args.subscription_id.trim().is_empty() {
                        return Err(FunctionCallError::RespondToModel(
                            "subscription_id must not be empty".to_string(),
                        ));
                    }
                    if args.chars.is_empty() {
                        return Err(FunctionCallError::RespondToModel(
                            "chars must not be empty".to_string(),
                        ));
                    }
                    self.registry
                        .write_event_command_stdin(self.thread_id, &args.subscription_id, &args.chars)
                        .await
                        .map_err(FunctionCallError::RespondToModel)?;
                    Ok(JsonToolOutput::new(json!(EventCommandWriteStdinResult {
                        subscription_id: args.subscription_id,
                        bytes_written: args.chars.len(),
                    })))
            })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use codex_extension_api::ToolOutput;
    use codex_extension_api::ToolPayload;
    use codex_file_watcher::FileWatcher;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::super::event_command_subscribe::EventCommandSubscribeTool;
    use super::*;
    use crate::runtime::UnavailableFileSubscriptionThreadRuntime;

    fn tool_call(arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            call_id: "call-1".to_string(),
            tool_name: ToolName::plain(TOOL_NAME),
            payload: ToolPayload::Function {
                arguments: arguments.to_string(),
            },
        }
    }

    fn test_registry() -> Arc<FsSubscriptionRegistry> {
        Arc::new(FsSubscriptionRegistry::new(
            Arc::new(FileWatcher::noop()),
            Arc::new(UnavailableFileSubscriptionThreadRuntime),
            None,
        ))
    }

    #[tokio::test]
    async fn rejects_empty_chars() {
        let tool = EventCommandWriteStdinTool {
            thread_id: ThreadId::new(),
            registry: test_registry(),
        };

        let err = tool
            .handle(tool_call(json!({
                "subscription_id": "sub-1",
                "chars": "",
            })))
            .await
            .expect_err("expected empty chars to be rejected");

        assert_eq!(
            err,
            FunctionCallError::RespondToModel("chars must not be empty".to_string())
        );
    }

    #[tokio::test]
    async fn writes_stdin_through_tool_executor() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("stdin.out");
        let thread_id = ThreadId::new();
        let registry = test_registry();
        let subscribe_tool = EventCommandSubscribeTool {
            thread_id,
            registry: Arc::clone(&registry),
        };
        let subscribe_payload = ToolPayload::Function {
            arguments: json!({
                "command": "IFS= read -r line; printf '%s' \"$line\" > stdin.out",
                "cwd": temp_dir.path().to_string_lossy(),
            })
            .to_string(),
        };
        let subscribe_output = subscribe_tool
            .handle(ToolCall {
                call_id: "subscribe-call".to_string(),
                tool_name: ToolName::plain("event_command_subscribe"),
                payload: subscribe_payload.clone(),
            })
            .await
            .expect("subscribe should succeed");
        let subscribe_response = subscribe_output
            .post_tool_use_response("subscribe-call", &subscribe_payload)
            .expect("subscribe response");
        let subscription_id = subscribe_response
            .get("subscription_id")
            .and_then(serde_json::Value::as_str)
            .expect("subscription_id")
            .to_string();

        let write_tool = EventCommandWriteStdinTool {
            thread_id,
            registry,
        };
        let payload = ToolPayload::Function {
            arguments: json!({
                "subscription_id": subscription_id,
                "chars": "hello tool\n",
            })
            .to_string(),
        };

        write_tool
            .handle(ToolCall {
                call_id: "call-1".to_string(),
                tool_name: ToolName::plain(TOOL_NAME),
                payload,
            })
            .await
            .expect("write should succeed");

        assert_eq!(read_file_eventually(&output_path).await, "hello tool");
    }

    async fn read_file_eventually(path: &std::path::Path) -> String {
        for _ in 0..20 {
            if let Ok(output) = std::fs::read_to_string(path) {
                return output;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("output file was not written");
    }
}
