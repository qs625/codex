use std::marker::PhantomData;

use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolExecutor;
use codex_tool_types::ToolExecutorFuture;
use codex_tool_types::ToolPayload;
use codex_tool_types::ToolSearchOutput;
use codex_tool_types::ToolSpec;

use codex_tool_planning::TOOL_SEARCH_DEFAULT_LIMIT;
use codex_tool_planning::TOOL_SEARCH_TOOL_NAME;
use codex_tool_planning::ToolSearchInfo;
use codex_tool_planning::ToolSearchRuntime;
use codex_tool_planning::create_tool_search_tool;
use codex_tool_runtime::ToolHandler;
use codex_tool_runtime::ToolInvocationView;
use codex_tool_types::ToolName;

pub struct ToolSearchHandler<Invocation> {
    runtime: ToolSearchRuntime,
    _marker: PhantomData<fn(Invocation)>,
}

impl<Invocation> ToolSearchHandler<Invocation> {
    pub fn new(search_infos: Vec<ToolSearchInfo>) -> Self {
        Self {
            runtime: ToolSearchRuntime::new(search_infos),
            _marker: PhantomData,
        }
    }
}

impl<Invocation> ToolExecutor<Invocation> for ToolSearchHandler<Invocation>
where
    Invocation: ToolInvocationView + Send,
{
    type Output = ToolSearchOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(TOOL_SEARCH_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_tool_search_tool(
            self.runtime.source_infos(),
            TOOL_SEARCH_DEFAULT_LIMIT,
        ))
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    fn handle<'a>(&'a self, invocation: Invocation) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
        Invocation: 'a,
    {
        Box::pin(async move {
            let ToolPayload::ToolSearch { arguments } = invocation.payload() else {
                return Err(FunctionCallError::Fatal(format!(
                    "{TOOL_SEARCH_TOOL_NAME} handler received unsupported payload"
                )));
            };

            self.runtime.handle_search(arguments.clone())
        })
    }
}

impl<Invocation, DiffContext> ToolHandler<Invocation, DiffContext> for ToolSearchHandler<Invocation> where
    Invocation: ToolInvocationView + Send
{
}

#[cfg(test)]
mod tests {
    use codex_protocol::models::SearchToolCallParams;
    use codex_tool_planning::TOOL_SEARCH_TOOL_NAME;
    use codex_tool_planning::ToolSearchInfo;
    use codex_tool_runtime::ToolInvocation;
    use codex_tool_types::ToolCallSource;
    use codex_tool_types::ToolExecutor;
    use codex_tool_types::ToolInvocationMetadata;
    use codex_tool_types::ToolName;
    use codex_tool_types::ToolPayload;
    use codex_tool_types::ToolSpec;

    use super::ToolSearchHandler;

    #[tokio::test]
    async fn handles_tool_search_payload() {
        let search_info = ToolSearchInfo::from_spec(
            "mail search".to_string(),
            ToolSpec::Function(codex_tool_types::ResponsesApiTool {
                name: "search_mail".to_string(),
                description: "Search mail".to_string(),
                strict: false,
                parameters: codex_tool_types::JsonSchema::object(
                    /*properties*/ Default::default(),
                    /*required*/ None,
                    /*additional_properties*/ None,
                ),
                output_schema: None,
                defer_loading: None,
            }),
            None,
        )
        .expect("search info");
        let handler = ToolSearchHandler::new(vec![search_info]);
        let invocation = ToolInvocation {
            session: (),
            turn: (),
            cancellation_token: tokio_util::sync::CancellationToken::new(),
            tracker: (),
            metadata: ToolInvocationMetadata {
                call_id: "call-search".to_string(),
                tool_name: ToolName::plain(TOOL_SEARCH_TOOL_NAME),
                source: ToolCallSource::Direct,
                payload: ToolPayload::ToolSearch {
                    arguments: SearchToolCallParams {
                        query: "mail".to_string(),
                        limit: Some(5),
                    },
                },
            },
        };

        let output = handler.handle(invocation).await.expect("search output");
        assert_eq!(output.tools.len(), 1);
    }
}
