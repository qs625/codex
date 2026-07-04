use crate::FunctionCallError;
use crate::ToolName;
use crate::ToolPayload;
use protocol::models::ResponseItem;
use protocol::models::SearchToolCallParams;

// TODO: this is temporary and will disappear in the next PR as
// codex-extension-api becomes generic on Invocation.
#[derive(Clone, Debug)]
pub struct ToolCall {
    pub call_id: String,
    pub tool_name: ToolName,
    pub payload: ToolPayload,
}

/// Host-neutral metadata for one concrete tool invocation.
///
/// This is the portion of a runtime invocation that can be shared by tool
/// routers, nested runtimes, tracing, hooks, and future handler crates without
/// depending on `codex-core` session or turn state.
#[derive(Clone, Debug)]
pub struct ToolInvocationMetadata {
    pub call_id: String,
    pub tool_name: ToolName,
    pub source: ToolCallSource,
    pub payload: ToolPayload,
}

/// Identifies the runtime source that requested a tool invocation.
///
/// This stays in `tool-types` so tool dispatchers and nested runtimes can
/// share the contract without depending on `codex-core`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolCallSource {
    Direct,
    CodeMode {
        /// Runtime cell that issued the nested tool request.
        cell_id: String,
        /// Code-mode's per-cell tool invocation id. This is useful for
        /// debugging the JS/runtime bridge, but it is not the Codex tool call id
        /// because the runtime id only needs to be unique within one cell.
        runtime_tool_call_id: String,
    },
}

impl ToolCall {
    pub fn into_invocation_metadata(self, source: ToolCallSource) -> ToolInvocationMetadata {
        let Self {
            call_id,
            tool_name,
            payload,
        } = self;
        ToolInvocationMetadata {
            call_id,
            tool_name,
            source,
            payload,
        }
    }

    pub fn from_response_item(item: ResponseItem) -> Result<Option<Self>, FunctionCallError> {
        match item {
            ResponseItem::FunctionCall {
                name,
                namespace,
                arguments,
                call_id,
                ..
            } => Ok(Some(Self {
                tool_name: ToolName::new(namespace, name),
                call_id,
                payload: ToolPayload::Function { arguments },
            })),
            ResponseItem::ToolSearchCall {
                call_id: Some(call_id),
                execution,
                arguments,
                ..
            } if execution == "client" => {
                let arguments: SearchToolCallParams =
                    serde_json::from_value(arguments).map_err(|err| {
                        FunctionCallError::RespondToModel(format!(
                            "failed to parse tool_search arguments: {err}"
                        ))
                    })?;
                Ok(Some(Self {
                    tool_name: ToolName::plain("tool_search"),
                    call_id,
                    payload: ToolPayload::ToolSearch { arguments },
                }))
            }
            ResponseItem::ToolSearchCall { .. } => Ok(None),
            ResponseItem::CustomToolCall {
                name,
                input,
                call_id,
                ..
            } => Ok(Some(Self {
                tool_name: ToolName::plain(name),
                call_id,
                payload: ToolPayload::Custom { input },
            })),
            _ => Ok(None),
        }
    }

    pub fn function_arguments(&self) -> Result<&str, FunctionCallError> {
        match &self.payload {
            ToolPayload::Function { arguments } => Ok(arguments),
            _ => Err(FunctionCallError::Fatal(format!(
                "tool {} invoked with incompatible payload",
                self.tool_name
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_function_tool_call_from_response_item() {
        let call = ToolCall::from_response_item(ResponseItem::FunctionCall {
            id: None,
            name: "exec_command".to_string(),
            namespace: None,
            arguments: "{\"cmd\":\"pwd\"}".to_string(),
            call_id: "call-1".to_string(),
        })
        .expect("function call should parse")
        .expect("function call should produce tool call");

        assert_eq!(call.call_id, "call-1");
        assert_eq!(call.tool_name, ToolName::plain("exec_command"));
        let ToolPayload::Function { arguments } = call.payload else {
            panic!("expected function payload");
        };
        assert_eq!(arguments, "{\"cmd\":\"pwd\"}");
    }

    #[test]
    fn builds_client_tool_search_call_from_response_item() {
        let call = ToolCall::from_response_item(ResponseItem::ToolSearchCall {
            id: None,
            call_id: Some("search-1".to_string()),
            status: None,
            execution: "client".to_string(),
            arguments: serde_json::json!({
                "query": "command tools",
                "limit": 3,
            }),
        })
        .expect("tool search call should parse")
        .expect("client tool search should produce tool call");

        assert_eq!(call.call_id, "search-1");
        assert_eq!(call.tool_name, ToolName::plain("tool_search"));
        let ToolPayload::ToolSearch { arguments } = call.payload else {
            panic!("expected tool search payload");
        };
        assert_eq!(arguments.query, "command tools");
        assert_eq!(arguments.limit, Some(3));
    }

    #[test]
    fn skips_non_client_tool_search_call() {
        let call = ToolCall::from_response_item(ResponseItem::ToolSearchCall {
            id: None,
            call_id: Some("search-1".to_string()),
            status: None,
            execution: "server".to_string(),
            arguments: serde_json::json!({
                "query": "command tools",
            }),
        })
        .expect("non-client tool search should not error");

        assert!(call.is_none());
    }

    #[test]
    fn builds_custom_tool_call_from_response_item() {
        let call = ToolCall::from_response_item(ResponseItem::CustomToolCall {
            id: None,
            status: None,
            call_id: "custom-1".to_string(),
            name: "code_mode".to_string(),
            input: "print(1)".to_string(),
        })
        .expect("custom tool call should parse")
        .expect("custom tool call should produce tool call");

        assert_eq!(call.call_id, "custom-1");
        assert_eq!(call.tool_name, ToolName::plain("code_mode"));
        let ToolPayload::Custom { input } = call.payload else {
            panic!("expected custom payload");
        };
        assert_eq!(input, "print(1)");
    }
}
