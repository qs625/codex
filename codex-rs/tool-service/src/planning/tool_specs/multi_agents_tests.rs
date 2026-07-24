use super::*;
use protocol::openai_models::ModelPreset;
use protocol::openai_models::ModelServiceTier;
use protocol::openai_models::ReasoningEffort;
use protocol::openai_models::ReasoningEffortPreset;
use serde_json::json;
use std::collections::BTreeMap;
use tool_service_api::JsonSchema;
use tool_service_api::JsonSchemaPrimitiveType;
use tool_service_api::JsonSchemaType;

fn function_tool(tool: ToolSpec, expected_name: &str) -> ResponsesApiTool {
    let ToolSpec::Function(tool) = tool else {
        panic!("{expected_name} should be a function tool");
    };
    assert_eq!(tool.name, expected_name);
    tool
}

fn property_shapes(tool: &ResponsesApiTool) -> BTreeMap<String, JsonSchema> {
    tool.parameters
        .properties
        .as_ref()
        .expect("function tool should use object params")
        .iter()
        .map(|(name, schema)| (name.clone(), schema_shape(schema)))
        .collect()
}

fn schema_shape(schema: &JsonSchema) -> JsonSchema {
    let mut shape = schema.clone();
    shape.description = None;
    shape.items = shape.items.map(|item| Box::new(schema_shape(&item)));
    shape.properties = shape.properties.map(|properties| {
        properties
            .into_iter()
            .map(|(name, schema)| (name, schema_shape(&schema)))
            .collect()
    });
    shape.any_of = shape.any_of.map(|variants| {
        variants
            .into_iter()
            .map(|schema| schema_shape(&schema))
            .collect()
    });
    shape
}

fn required_params(tool: &ResponsesApiTool) -> Vec<String> {
    tool.parameters.required.clone().unwrap_or_default()
}

fn assert_object_params(tool: &ResponsesApiTool) {
    assert_eq!(
        tool.parameters.schema_type,
        Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Object))
    );
}

fn model_preset(id: &str, show_in_picker: bool) -> ModelPreset {
    ModelPreset {
        id: id.to_string(),
        model: format!("{id}-model"),
        display_name: format!("{id} display"),
        description: format!("{id} description"),
        default_reasoning_effort: ReasoningEffort::Medium,
        supported_reasoning_efforts: vec![ReasoningEffortPreset {
            effort: ReasoningEffort::Medium,
            description: "Balanced".to_string(),
        }],
        supports_personality: false,
        additional_speed_tiers: Vec::new(),
        service_tiers: vec![ModelServiceTier {
            id: "priority".to_string(),
            name: "Fast".to_string(),
            description: "1.5x speed, increased usage".to_string(),
        }],
        is_default: false,
        upgrade: None,
        show_in_picker,
        availability_nux: None,
        supported_in_api: true,
        input_modalities: Vec::new(),
        context_window: None,
        max_context_window: None,
        auto_compact_token_limit: None,
    }
}

#[test]
fn spawn_agent_tool_v2_requires_task_name_and_lists_visible_models() {
    let tool = create_spawn_agent_tool_v2(SpawnAgentToolOptions {
        available_models: vec![
            model_preset("visible", /*show_in_picker*/ true),
            model_preset("hidden", /*show_in_picker*/ false),
        ],
        agent_type_description: "role help".to_string(),
        hide_agent_type_model_reasoning: false,
        include_usage_hint: true,
        usage_hint_text: None,
        max_concurrent_threads_per_session: Some(4),
    });

    let ToolSpec::Function(ResponsesApiTool {
        description,
        parameters,
        output_schema,
        ..
    }) = tool
    else {
        panic!("spawn_agent should be a function tool");
    };
    assert_eq!(
        parameters.schema_type,
        Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Object))
    );
    let properties = parameters
        .properties
        .as_ref()
        .expect("spawn_agent should use object params");
    assert!(description.contains("Spawns an agent to work on the specified task."));
    assert!(description.contains("The spawned agent will have the same tools as you"));
    assert!(description.contains("`max_concurrent_threads_per_session = 4`"));
    assert!(description.contains(SPAWN_AGENT_INHERITED_MODEL_GUIDANCE));
    assert!(
        description
            .contains("Available model overrides (optional; inherited parent model is preferred):")
    );
    assert!(description.contains("visible display (`visible-model`)"));
    assert!(
        description
            .contains("Supported service tiers: priority (Fast: 1.5x speed, increased usage).")
    );
    assert!(!description.contains("hidden display (`hidden-model`)"));
    assert!(properties.contains_key("task_name"));
    assert!(properties.contains_key("message"));
    assert!(properties.contains_key("cwd"));
    assert!(!properties.contains_key("provider"));
    assert!(properties.contains_key("fork_turns"));
    assert!(!properties.contains_key("agent_mode"));
    assert!(!properties.contains_key("items"));
    assert!(!properties.contains_key("fork_context"));
    let agent_type_description = properties
        .get("agent_type")
        .and_then(|schema| schema.description.as_deref())
        .expect("agent_type description");
    assert!(agent_type_description.contains("role help"));
    assert!(
        agent_type_description
            .contains("When `cwd` is set, agent types from that cwd or its repository may be used")
    );
    assert_eq!(
        properties
            .get("model")
            .and_then(|schema| schema.description.as_deref()),
        Some(SPAWN_AGENT_MODEL_OVERRIDE_DESCRIPTION)
    );
    assert_eq!(
        properties
            .get("service_tier")
            .and_then(|schema| schema.description.as_deref()),
        Some(SPAWN_AGENT_SERVICE_TIER_OVERRIDE_DESCRIPTION)
    );
    assert_eq!(
        parameters.required.as_ref(),
        Some(&vec!["task_name".to_string(), "message".to_string()])
    );
    assert_eq!(
        output_schema.expect("spawn_agent output schema")["required"],
        json!(["task_name", "nickname"])
    );
}

#[test]
fn spawn_external_agent_tool_requires_provider_cwd_and_message() {
    let tool = function_tool(create_spawn_external_agent_tool(), "spawn_external_agent");
    let ResponsesApiTool {
        description,
        parameters,
        output_schema,
        ..
    } = tool;
    assert!(description.contains("external code-agent CLI"));
    assert!(description.contains("spawn_agent only for Morpheus native"));
    let properties = parameters
        .properties
        .as_ref()
        .expect("spawn_external_agent should use object params");
    assert_eq!(
        properties
            .get("provider")
            .and_then(|schema| schema.enum_values.as_ref()),
        Some(&vec![
            json!("claude_cli"),
            json!("opencode"),
            json!("codex_cli")
        ])
    );
    assert!(
        !properties
            .get("provider")
            .and_then(|schema| schema.enum_values.as_ref())
            .is_some_and(|values| values.contains(&json!("native"))),
        "native should not be exposed through the external spawn surface"
    );
    assert!(properties.contains_key("task_name"));
    assert!(properties.contains_key("cwd"));
    assert!(properties.contains_key("message"));
    assert_eq!(
        parameters.required.as_ref(),
        Some(&vec![
            "task_name".to_string(),
            "provider".to_string(),
            "cwd".to_string(),
            "message".to_string(),
        ])
    );
    assert_eq!(
        output_schema.expect("spawn_external_agent output schema")["required"],
        json!(["task_name", "nickname"])
    );
}

#[test]
fn external_and_native_followup_tools_share_parameter_shape() {
    let native = function_tool(create_followup_task_tool(), "followup_task");
    let external = function_tool(
        create_followup_external_task_tool(),
        "followup_external_task",
    );

    assert_object_params(&native);
    assert_object_params(&external);
    assert_eq!(property_shapes(&external), property_shapes(&native));
    assert_eq!(required_params(&external), required_params(&native));
    assert_eq!(native.output_schema, None);
    assert_eq!(external.output_schema, None);
}

#[test]
fn external_and_native_list_tools_share_parameter_shape() {
    let native = function_tool(create_list_agents_tool(), "list_agents");
    let external = function_tool(create_list_external_agents_tool(), "list_external_agents");

    assert_object_params(&native);
    assert_object_params(&external);
    assert_eq!(property_shapes(&external), property_shapes(&native));
    assert_eq!(required_params(&external), required_params(&native));
    assert_eq!(external.output_schema, native.output_schema);
}

#[test]
fn external_and_native_close_tools_share_parameter_shape() {
    let native = function_tool(create_close_agent_tool_v2(), "close_agent");
    let external = function_tool(create_close_external_agent_tool(), "close_external_agent");

    assert_object_params(&native);
    assert_object_params(&external);
    assert_eq!(property_shapes(&external), property_shapes(&native));
    assert_eq!(required_params(&external), required_params(&native));
    assert_eq!(external.output_schema, native.output_schema);
}

#[test]
fn external_poll_keeps_empty_params_and_matches_native_wake_metadata() {
    let native = function_tool(create_poll_event_tool(), "poll_event");
    let external = function_tool(create_poll_external_event_tool(), "poll_external_event");

    assert_object_params(&native);
    assert_object_params(&external);
    assert_eq!(property_shapes(&external), property_shapes(&native));
    assert_eq!(required_params(&external), required_params(&native));
    assert_eq!(external.output_schema, native.output_schema);
}

#[test]
fn poll_event_output_schema_matches_thread_poll_event_result_json_keys() {
    let tool = function_tool(create_poll_event_tool(), "poll_event");
    let output_schema = tool.output_schema.expect("poll_event output schema");
    let serialized = serde_json::to_value(thread_service_api::ThreadPollEventResult {
        timed_out: false,
        source_hint: Some("inter_agent".to_string()),
        event: None,
        events: Vec::new(),
        waited_ms: 1,
        initial_timeout_ms: 10,
        current_timeout_ms: 10,
        hard_cap_timeout_ms: 20,
    })
    .expect("serialize poll event result");
    let mut schema_keys = output_schema["required"]
        .as_array()
        .expect("required keys")
        .iter()
        .map(|key| key.as_str().expect("string key"))
        .collect::<Vec<_>>();
    let mut serialized_keys = serialized
        .as_object()
        .expect("serialized object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    schema_keys.sort_unstable();
    serialized_keys.sort_unstable();

    assert_eq!(schema_keys, serialized_keys);
}

#[test]
fn spawn_external_agent_shares_common_fields_but_excludes_native_only_options() {
    let native = function_tool(
        create_spawn_agent_tool_v2(SpawnAgentToolOptions {
            available_models: vec![model_preset("visible", /*show_in_picker*/ true)],
            agent_type_description: "role help".to_string(),
            hide_agent_type_model_reasoning: false,
            include_usage_hint: false,
            usage_hint_text: None,
            max_concurrent_threads_per_session: None,
        }),
        "spawn_agent",
    );
    let external = function_tool(create_spawn_external_agent_tool(), "spawn_external_agent");
    let native_properties = native
        .parameters
        .properties
        .as_ref()
        .expect("spawn_agent should use object params");
    let external_properties = external
        .parameters
        .properties
        .as_ref()
        .expect("spawn_external_agent should use object params");

    for common_field in ["task_name", "cwd", "message"] {
        assert!(
            native_properties.contains_key(common_field),
            "native spawn should include common field {common_field}"
        );
        assert!(
            external_properties.contains_key(common_field),
            "external spawn should include common field {common_field}"
        );
        assert_eq!(
            external_properties
                .get(common_field)
                .and_then(|schema| schema.schema_type.as_ref()),
            native_properties
                .get(common_field)
                .and_then(|schema| schema.schema_type.as_ref()),
            "common field {common_field} should keep the same primitive shape"
        );
    }

    assert_eq!(
        required_params(&external),
        vec![
            "task_name".to_string(),
            "provider".to_string(),
            "cwd".to_string(),
            "message".to_string()
        ]
    );
    assert_eq!(
        required_params(&native),
        vec!["task_name".to_string(), "message".to_string()]
    );
    assert!(external_properties.contains_key("provider"));
    for native_only_field in [
        "agent_type",
        "model",
        "reasoning_effort",
        "service_tier",
        "fork_turns",
    ] {
        assert!(
            native_properties.contains_key(native_only_field),
            "native spawn should keep native-only field {native_only_field}"
        );
        assert!(
            !external_properties.contains_key(native_only_field),
            "external spawn should not expose native-only field {native_only_field}"
        );
    }
}

#[test]
fn spawn_agent_tool_hides_service_tier_with_spawn_metadata() {
    let tool = create_spawn_agent_tool_v2(SpawnAgentToolOptions {
        available_models: vec![model_preset("visible", /*show_in_picker*/ true)],
        agent_type_description: "role help".to_string(),
        hide_agent_type_model_reasoning: true,
        include_usage_hint: true,
        usage_hint_text: None,
        max_concurrent_threads_per_session: Some(4),
    });

    let ToolSpec::Function(ResponsesApiTool { parameters, .. }) = tool else {
        panic!("spawn_agent should be a function tool");
    };
    let properties = parameters
        .properties
        .as_ref()
        .expect("spawn_agent should use object params");

    assert!(!properties.contains_key("agent_type"));
    assert!(!properties.contains_key("model"));
    assert!(!properties.contains_key("reasoning_effort"));
    assert!(!properties.contains_key("service_tier"));
}

#[test]
fn followup_task_tool_requires_message_and_has_no_output_schema() {
    let ToolSpec::Function(ResponsesApiTool {
        parameters,
        output_schema,
        ..
    }) = create_followup_task_tool()
    else {
        panic!("followup_task should be a function tool");
    };
    assert_eq!(
        parameters.schema_type,
        Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Object))
    );
    let properties = parameters
        .properties
        .as_ref()
        .expect("followup_task should use object params");
    assert!(properties.contains_key("target"));
    assert!(properties.contains_key("message"));
    assert!(!properties.contains_key("items"));
    assert_eq!(
        parameters.required.as_ref(),
        Some(&vec!["target".to_string(), "message".to_string()])
    );
    assert_eq!(output_schema, None);
}

#[test]
fn poll_event_tool_has_empty_object_params_and_optional_payload() {
    let ToolSpec::Function(ResponsesApiTool {
        parameters,
        output_schema,
        ..
    }) = create_poll_event_tool()
    else {
        panic!("poll_event should be a function tool");
    };
    assert_eq!(
        parameters.schema_type,
        Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Object))
    );
    assert_eq!(parameters.required.as_ref(), Some(&Vec::<String>::new()));
    let output_schema = output_schema.expect("poll_event output schema");
    assert_eq!(
        output_schema["required"],
        json!([
            "timedOut",
            "sourceHint",
            "waitedMs",
            "initialTimeoutMs",
            "currentTimeoutMs",
            "hardCapTimeoutMs"
        ])
    );
    assert!(
        output_schema["properties"]["event"].is_object(),
        "poll_event should document its optional typed payload"
    );
    assert!(
        output_schema["properties"]["events"].is_object(),
        "poll_event should document the visible typed payload list"
    );
}

#[test]
fn list_agents_tool_includes_path_prefix_and_agent_fields() {
    let ToolSpec::Function(ResponsesApiTool {
        parameters,
        output_schema,
        ..
    }) = create_list_agents_tool()
    else {
        panic!("list_agents should be a function tool");
    };
    assert_eq!(
        parameters.schema_type,
        Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Object))
    );
    let properties = parameters
        .properties
        .as_ref()
        .expect("list_agents should use object params");
    assert!(properties.contains_key("path_prefix"));
    assert_eq!(
        properties
            .get("path_prefix")
            .and_then(|schema| schema.description.as_deref()),
        Some(
            "Optional task-path prefix (not ending with trailing slash). Accepts the same relative or absolute task-path syntax."
        )
    );
    assert_eq!(
        output_schema.expect("list_agents output schema")["properties"]["agents"]["items"]["required"],
        json!(["agent_name", "lifecycle_status", "last_task_message"])
    );
}

#[test]
fn list_agents_tool_lifecycle_schema_includes_interrupted_final() {
    let ToolSpec::Function(ResponsesApiTool { output_schema, .. }) = create_list_agents_tool()
    else {
        panic!("list_agents should be a function tool");
    };

    assert_eq!(
        output_schema.expect("list_agents output schema")["properties"]["agents"]["items"]["properties"]
            ["lifecycle_status"]["allOf"][0]["oneOf"][4]["properties"]["result"]["oneOf"][2]["properties"]
            ["type"]["enum"],
        json!(["interrupted"])
    );
}
