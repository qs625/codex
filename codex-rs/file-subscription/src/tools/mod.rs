use std::sync::Arc;

use codex_core::UnifiedExecProcessManager;
use codex_extension_api::ExtensionToolExecutor;
use codex_extension_api::FunctionCallError;
use codex_extension_api::ResponsesApiTool;
use codex_extension_api::ToolCall;
use codex_extension_api::ToolSpec;
use codex_extension_api::parse_tool_input_schema;
use codex_protocol::ThreadId;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::registry::FsSubscriptionRegistry;
use crate::schema::input_schema_for;

mod process_exit_subscribe;
mod process_exit_unsubscribe;
mod subscribe;
mod timer_subscribe;
mod timer_unsubscribe;
mod unsubscribe;

pub(crate) fn subscription_tools(
    thread_id: ThreadId,
    registry: Arc<FsSubscriptionRegistry>,
    unified_exec_manager: Option<Arc<UnifiedExecProcessManager>>,
) -> Vec<Arc<dyn ExtensionToolExecutor>> {
    let shared_registry = Arc::clone(&registry);
    let mut tools: Vec<Arc<dyn ExtensionToolExecutor>> = vec![
        Arc::new(subscribe::FsSubscribeTool {
            thread_id,
            registry: Arc::clone(&shared_registry),
        }),
        Arc::new(unsubscribe::FsUnsubscribeTool {
            thread_id,
            registry: Arc::clone(&shared_registry),
        }),
        Arc::new(timer_subscribe::TimerSubscribeTool {
            thread_id,
            registry: Arc::clone(&shared_registry),
        }),
        Arc::new(timer_unsubscribe::TimerUnsubscribeTool {
            thread_id,
            registry: Arc::clone(&shared_registry),
        }),
    ];
    if let Some(unified_exec_manager) = unified_exec_manager {
        tools.push(Arc::new(process_exit_subscribe::ProcessExitSubscribeTool {
            thread_id,
            registry: Arc::clone(&shared_registry),
            unified_exec_manager,
        }));
        tools.push(Arc::new(
            process_exit_unsubscribe::ProcessExitUnsubscribeTool {
                thread_id,
                registry: Arc::clone(&shared_registry),
            },
        ));
    }
    tools
}

pub(super) fn subscription_function_tool<I: JsonSchema>(name: &str, description: &str) -> ToolSpec {
    let parameters = parse_tool_input_schema(&input_schema_for::<I>())
        .unwrap_or_else(|err| panic!("generated input schema for {name} should parse: {err}"));
    ToolSpec::Function(ResponsesApiTool {
        name: name.to_string(),
        description: description.to_string(),
        strict: false,
        defer_loading: None,
        parameters,
        output_schema: None,
    })
}

pub(super) fn parse_args<T: for<'de> Deserialize<'de>>(
    call: &ToolCall,
) -> Result<T, FunctionCallError> {
    let arguments = call.function_arguments()?;
    let value = if arguments.trim().is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(arguments)
            .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?
    };
    serde_json::from_value(value).map_err(|err| FunctionCallError::RespondToModel(err.to_string()))
}
