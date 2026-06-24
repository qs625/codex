use std::marker::PhantomData;
use std::sync::Arc;

use codex_extension_api::ExtensionToolExecutor;
use codex_extension_api::ExtensionToolOutput;
use codex_tool_types::ToolCall;
use codex_tool_types::ToolExecutor;
use codex_tool_types::ToolExecutorFuture;
use codex_tool_types::ToolName;
use codex_tool_types::ToolOutput;
use codex_tool_types::ToolPayload;
use codex_tool_types::ToolSpec;
use serde_json::Value;

use crate::HookToolName;
use crate::PostToolUsePayload;
use crate::PreToolUsePayload;
use crate::flat_tool_name;
use codex_tool_runtime::ToolHandler;
use codex_tool_runtime::ToolInvocationView;

pub struct ExtensionToolHandler<Invocation> {
    executor: Arc<dyn ExtensionToolExecutor>,
    _marker: PhantomData<fn(Invocation)>,
}

impl<Invocation> ExtensionToolHandler<Invocation> {
    pub fn new(executor: Arc<dyn ExtensionToolExecutor>) -> Self {
        Self {
            executor,
            _marker: PhantomData,
        }
    }

    fn arguments_from_payload<'a>(&self, payload: &'a ToolPayload) -> Option<&'a str> {
        let ToolPayload::Function { arguments } = payload else {
            return None;
        };
        Some(arguments)
    }
}

impl<Invocation> ToolExecutor<Invocation> for ExtensionToolHandler<Invocation>
where
    Invocation: ToolInvocationView + Send,
{
    type Output = ExtensionToolOutput;

    fn tool_name(&self) -> ToolName {
        self.executor.tool_name()
    }

    fn spec(&self) -> Option<ToolSpec> {
        self.executor.spec()
    }

    fn handle<'a>(&'a self, invocation: Invocation) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
        Invocation: 'a,
    {
        Box::pin(async move { self.executor.handle(to_extension_call(&invocation)).await })
    }
}

impl<Invocation, DiffContext> ToolHandler<Invocation, DiffContext>
    for ExtensionToolHandler<Invocation>
where
    Invocation: ToolInvocationView + Send,
{
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        self.arguments_from_payload(payload).is_some()
    }

    fn pre_tool_use_payload(&self, invocation: &Invocation) -> Option<PreToolUsePayload> {
        let arguments = self.arguments_from_payload(invocation.payload())?;
        Some(PreToolUsePayload {
            tool_name: HookToolName::new(flat_tool_name(&self.tool_name()).into_owned()),
            tool_input: extension_tool_hook_input(arguments),
        })
    }

    fn post_tool_use_payload(
        &self,
        invocation: &Invocation,
        result: &Self::Output,
    ) -> Option<PostToolUsePayload> {
        let arguments = self.arguments_from_payload(invocation.payload())?;
        Some(PostToolUsePayload {
            tool_name: HookToolName::new(flat_tool_name(&self.tool_name()).into_owned()),
            tool_use_id: invocation.call_id().to_string(),
            tool_input: extension_tool_hook_input(arguments),
            tool_response: result
                .post_tool_use_response(invocation.call_id(), invocation.payload())?,
        })
    }
}

fn to_extension_call(invocation: &impl ToolInvocationView) -> ToolCall {
    ToolCall {
        call_id: invocation.call_id().to_string(),
        tool_name: invocation.tool_name().clone(),
        payload: invocation.payload().clone(),
    }
}

fn extension_tool_hook_input(arguments: &str) -> Value {
    if arguments.trim().is_empty() {
        return Value::Object(serde_json::Map::new());
    }

    serde_json::from_str(arguments).unwrap_or_else(|_| Value::String(arguments.to_string()))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::ExtensionToolHandler;
    use crate::HookToolName;
    use crate::PostToolUsePayload;
    use crate::PreToolUsePayload;
    use codex_tool_runtime::ToolHandler;
    use codex_tool_runtime::ToolInvocation;
    use codex_tool_types::JsonToolOutput;
    use codex_tool_types::ResponsesApiTool;
    use codex_tool_types::ToolCall;
    use codex_tool_types::ToolCallSource;
    use codex_tool_types::ToolExecutor;
    use codex_tool_types::ToolExecutorFuture;
    use codex_tool_types::ToolInvocationMetadata;
    use codex_tool_types::ToolName;
    use codex_tool_types::ToolPayload;
    use codex_tool_types::ToolSpec;
    use codex_tool_types::parse_tool_input_schema;

    struct StubExtensionExecutor;

    impl ToolExecutor<ToolCall> for StubExtensionExecutor {
        type Output = JsonToolOutput;

        fn tool_name(&self) -> ToolName {
            ToolName::plain("extension_echo")
        }

        fn spec(&self) -> Option<ToolSpec> {
            Some(ToolSpec::Function(ResponsesApiTool {
                name: "extension_echo".to_string(),
                description: "Echoes arguments.".to_string(),
                strict: true,
                parameters: parse_tool_input_schema(&json!({
                    "type": "object",
                    "properties": {
                        "message": { "type": "string" },
                    },
                    "required": ["message"],
                    "additionalProperties": false,
                }))
                .expect("extension schema should parse"),
                output_schema: None,
                defer_loading: None,
            }))
        }

        fn handle<'a>(&'a self, _call: ToolCall) -> ToolExecutorFuture<'a, Self::Output>
        where
            Self: 'a,
        {
            Box::pin(async move { Ok(JsonToolOutput::new(json!({ "ok": true }))) })
        }
    }

    #[test]
    fn exposes_generic_hook_payloads() {
        let handler = ExtensionToolHandler::new(std::sync::Arc::new(StubExtensionExecutor));
        let invocation = ToolInvocation {
            session: (),
            turn: (),
            cancellation_token: tokio_util::sync::CancellationToken::new(),
            tracker: (),
            metadata: ToolInvocationMetadata {
                call_id: "call-extension".to_string(),
                tool_name: ToolName::plain("extension_echo"),
                source: ToolCallSource::Direct,
                payload: ToolPayload::Function {
                    arguments: json!({ "message": "hello" }).to_string(),
                },
            },
        };
        let output = JsonToolOutput::new(json!({ "ok": true }));

        assert_eq!(
            ToolHandler::<_, ()>::pre_tool_use_payload(&handler, &invocation),
            Some(PreToolUsePayload {
                tool_name: HookToolName::new("extension_echo"),
                tool_input: json!({ "message": "hello" }),
            })
        );
        assert_eq!(
            ToolHandler::<_, ()>::post_tool_use_payload(&handler, &invocation, &output),
            Some(PostToolUsePayload {
                tool_name: HookToolName::new("extension_echo"),
                tool_use_id: "call-extension".to_string(),
                tool_input: json!({ "message": "hello" }),
                tool_response: json!({ "ok": true }),
            })
        );
    }
}
