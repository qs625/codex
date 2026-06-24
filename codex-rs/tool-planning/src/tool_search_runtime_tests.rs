use super::*;
use crate::JsonSchema;
use crate::ResponsesApiNamespace;
use crate::ResponsesApiNamespaceTool;
use crate::ResponsesApiTool;
use crate::ToolSpec;
use codex_protocol::models::SearchToolCallParams;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;

#[test]
fn search_index_matches_underscore_terms_with_space_query() {
    let runtime = ToolSearchRuntime::new(vec![
        search_info(
            "name quasar_ping_beacon namespace orbit_ops",
            ToolSpec::Function(function_tool("quasar_ping_beacon", "Ping")),
        ),
        search_info(
            "name calendar_timezone_option_99 namespace calendar",
            ToolSpec::Function(function_tool("calendar_timezone_option_99", "Timezone")),
        ),
    ]);

    assert_eq!(
        result_names(&runtime, "quasar ping beacon", Some(1)),
        vec!["quasar_ping_beacon"]
    );
    assert_eq!(
        result_names(&runtime, "calendar_timezone_option_99", Some(1)),
        vec!["calendar_timezone_option_99"]
    );
}

#[test]
fn search_index_matches_description_and_schema_terms() {
    let runtime = ToolSearchRuntime::new(vec![
        search_info(
            "description Extract text from uploaded documents",
            ToolSpec::Function(function_tool("extract_text", "Extract text")),
        ),
        search_info(
            "schema starts_at title",
            ToolSpec::Function(function_tool("create_event", "Create event")),
        ),
        search_info(
            "description Delete archived records",
            ToolSpec::Function(function_tool("delete_records", "Delete records")),
        ),
    ]);

    assert_eq!(
        result_names(&runtime, "uploaded document", Some(1)),
        vec!["extract_text"]
    );
    assert_eq!(
        result_names(&runtime, "starts_at", Some(1)),
        vec!["create_event"]
    );
}

#[test]
fn mixed_search_results_coalesce_namespaces() {
    let runtime = ToolSearchRuntime::new(vec![
        search_info(
            "calendar create event",
            namespace_spec(
                "mcp__calendar__",
                "Calendar tools",
                vec![function_tool("create_event", "Create events")],
            ),
        ),
        search_info(
            "automation update recurring",
            namespace_spec(
                "codex_app",
                "App tools",
                vec![function_tool(
                    "automation_update",
                    "Create, update, view, or delete recurring automations.",
                )],
            ),
        ),
        search_info(
            "calendar list events",
            namespace_spec(
                "mcp__calendar__",
                "Calendar tools",
                vec![function_tool("list_events", "List events")],
            ),
        ),
    ]);

    let tools = runtime
        .search_output_tools([
            &runtime.entries[0],
            &runtime.entries[2],
            &runtime.entries[1],
        ])
        .expect("search output should coalesce");

    assert_eq!(
        tools,
        vec![
            crate::LoadableToolSpec::Namespace(ResponsesApiNamespace {
                name: "mcp__calendar__".to_string(),
                description: "Calendar tools".to_string(),
                tools: vec![
                    ResponsesApiNamespaceTool::Function(loadable_function_tool(
                        "create_event",
                        "Create events",
                    )),
                    ResponsesApiNamespaceTool::Function(loadable_function_tool(
                        "list_events",
                        "List events",
                    )),
                ],
            }),
            crate::LoadableToolSpec::Namespace(ResponsesApiNamespace {
                name: "codex_app".to_string(),
                description: "App tools".to_string(),
                tools: vec![ResponsesApiNamespaceTool::Function(loadable_function_tool(
                    "automation_update",
                    "Create, update, view, or delete recurring automations.",
                ))],
            }),
        ]
    );
}

fn result_names(runtime: &ToolSearchRuntime, query: &str, limit: Option<usize>) -> Vec<String> {
    runtime
        .handle_search(SearchToolCallParams {
            query: query.to_string(),
            limit,
        })
        .expect("search should succeed")
        .tools
        .into_iter()
        .map(|tool| match tool {
            crate::LoadableToolSpec::Function(tool) => tool.name,
            crate::LoadableToolSpec::Namespace(namespace) => namespace.name,
        })
        .collect()
}

fn search_info(search_text: &str, spec: ToolSpec) -> ToolSearchInfo {
    ToolSearchInfo::from_spec(search_text.to_string(), spec, None)
        .expect("spec should produce search info")
}

fn namespace_spec(name: &str, description: &str, tools: Vec<ResponsesApiTool>) -> ToolSpec {
    ToolSpec::Namespace(ResponsesApiNamespace {
        name: name.to_string(),
        description: description.to_string(),
        tools: tools
            .into_iter()
            .map(ResponsesApiNamespaceTool::Function)
            .collect(),
    })
}

fn function_tool(name: &str, description: &str) -> ResponsesApiTool {
    ResponsesApiTool {
        name: name.to_string(),
        description: description.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(BTreeMap::new(), /*required*/ None, Some(false.into())),
        output_schema: Some(serde_json::json!({})),
    }
}

fn loadable_function_tool(name: &str, description: &str) -> ResponsesApiTool {
    ResponsesApiTool {
        defer_loading: Some(true),
        output_schema: None,
        ..function_tool(name, description)
    }
}
