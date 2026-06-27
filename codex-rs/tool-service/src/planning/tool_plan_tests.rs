use super::*;
use crate::JsonSchema;
use crate::ResponsesApiNamespace;
use crate::ResponsesApiTool;
use crate::ToolName;
use codex_features::Feature;
use codex_features::Features;
use codex_protocol::config_types::WebSearchMode;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::PermissionProfile;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::protocol::SessionSource;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn hosted_model_tool_specs_include_web_search_and_image_generation() {
    let mut features = Features::with_defaults();
    features.enable(Feature::ImageGeneration);
    let model_info = model_info();
    let available_models = Vec::new();
    let config = ToolsConfig::new(&crate::ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Live),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    assert_eq!(
        hosted_model_tool_specs(&config),
        vec![
            ToolSpec::WebSearch {
                external_web_access: Some(true),
                filters: None,
                user_location: None,
                search_context_size: None,
                search_content_types: None,
            },
            ToolSpec::ImageGeneration {
                output_format: "png".to_string(),
            },
        ]
    );
}

#[test]
fn filter_tool_specs_for_agent_filters_namespace_children() {
    let config =
        tools_config_with_agent_tool_patterns(vec!["test_server/do_something_cool".to_string()]);
    let specs = vec![namespace_spec(
        "test_server/",
        "",
        vec![
            function_tool("do_something_cool", "Do something cool"),
            function_tool("delete_everything", "Delete everything"),
        ],
    )];

    assert_eq!(
        filter_tool_specs_for_agent(&config, specs),
        vec![namespace_spec(
            "test_server/",
            "",
            vec![function_tool("do_something_cool", "Do something cool")]
        )]
    );
}

#[test]
fn filter_tool_specs_for_agent_keeps_whole_namespace_on_namespace_match() {
    let config = tools_config_with_agent_tool_patterns(vec!["test_server/*".to_string()]);
    let specs = vec![namespace_spec(
        "test_server/",
        "",
        vec![
            function_tool("do_something_cool", "Do something cool"),
            function_tool("delete_everything", "Delete everything"),
        ],
    )];

    assert_eq!(filter_tool_specs_for_agent(&config, specs.clone()), specs);
}

#[test]
fn merge_tool_specs_into_namespaces_merges_sorts_and_fills_description() {
    assert_eq!(
        merge_tool_specs_into_namespaces(vec![
            namespace_spec("test_server/", "", vec![function_tool("zebra", "z")]),
            ToolSpec::Function(function_tool("plain", "Plain tool")),
            namespace_spec(
                "test_server/",
                "Test server tools.",
                vec![function_tool("alpha", "a")]
            ),
            namespace_spec("empty_server/", "", vec![function_tool("echo", "Echo")]),
        ]),
        vec![
            namespace_spec(
                "test_server/",
                "Test server tools.",
                vec![function_tool("alpha", "a"), function_tool("zebra", "z")]
            ),
            ToolSpec::Function(function_tool("plain", "Plain tool")),
            namespace_spec(
                "empty_server/",
                "Tools in the empty_server/ namespace.",
                vec![function_tool("echo", "Echo")]
            ),
        ]
    );
}

#[test]
fn code_mode_exec_plan_sorts_namespaced_tools_after_plain_tools() {
    let specs = vec![
        namespace_spec(
            "beta_",
            "Beta tools",
            vec![
                function_tool("zulu", "Zulu"),
                function_tool("alpha", "Alpha"),
            ],
        ),
        ToolSpec::Function(function_tool("plain", "Plain tool")),
        namespace_spec("alpha_", "Alpha tools", vec![function_tool("echo", "Echo")]),
    ];

    let plan = code_mode_exec_plan_for_specs(&specs);

    assert_eq!(
        plan.enabled_tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        vec!["plain", "alpha_echo", "beta_alpha", "beta_zulu"]
    );
    assert_eq!(
        plan.namespace_descriptions
            .get("alpha_")
            .expect("alpha namespace description")
            .description,
        "Alpha tools"
    );
}

#[test]
fn plan_tool_registry_entries_splits_host_entries_from_model_visible_specs() {
    let mut config = tools_config_with_agent_tool_patterns(vec![
        "safe*".to_string(),
        "mcp__server__allowed".to_string(),
    ]);
    config.code_mode_enabled = true;
    config.search_tool = true;
    config.namespace_tools = true;

    let deferred_search_info = ToolSearchInfo::from_spec(
        "allowed deferred tool".to_string(),
        ToolSpec::Function(function_tool("allowed", "Allowed deferred tool")),
        None,
    )
    .expect("function spec should be searchable");
    let entries = vec![
        TestRegistryEntry {
            name: ToolName::plain("safe_direct"),
            exposure: ToolExposure::Direct,
            spec: Some(ToolSpec::Function(function_tool(
                "safe_direct",
                "Direct tool",
            ))),
            search_info: None,
        },
        TestRegistryEntry {
            name: ToolName::plain("safe_model_only"),
            exposure: ToolExposure::DirectModelOnly,
            spec: Some(ToolSpec::Function(function_tool(
                "safe_model_only",
                "Model only tool",
            ))),
            search_info: None,
        },
        TestRegistryEntry {
            name: ToolName::namespaced("mcp__server__", "allowed"),
            exposure: ToolExposure::Deferred,
            spec: Some(namespace_spec(
                "mcp__server__",
                "Server tools",
                vec![function_tool("allowed", "Allowed deferred tool")],
            )),
            search_info: Some(deferred_search_info.clone()),
        },
        TestRegistryEntry {
            name: ToolName::plain("blocked"),
            exposure: ToolExposure::Direct,
            spec: Some(ToolSpec::Function(function_tool("blocked", "Blocked tool"))),
            search_info: None,
        },
    ];

    let plan = plan_tool_registry_entries(
        &config,
        entries,
        vec![ToolSpec::Function(function_tool(
            "safe_hosted",
            "Hosted tool",
        ))],
    );

    assert_eq!(
        plan.entries
            .iter()
            .map(|entry| entry.name.to_string())
            .collect::<Vec<_>>(),
        vec!["safe_direct", "safe_model_only", "mcp__server__allowed"]
    );
    assert_eq!(plan.deferred_search_infos, vec![deferred_search_info]);
    assert!(plan.deferred_tools_available);

    let model_visible_names = plan
        .model_visible_specs
        .iter()
        .map(ToolSpec::name)
        .collect::<Vec<_>>();
    assert_eq!(
        model_visible_names,
        vec!["safe_direct", "safe_model_only", "safe_hosted"]
    );

    let code_mode_nested_names = plan
        .code_mode_nested_tool_specs
        .iter()
        .map(ToolSpec::name)
        .collect::<Vec<_>>();
    assert_eq!(code_mode_nested_names, vec!["safe_direct", "mcp__server__"]);
}

#[derive(Clone)]
struct TestRegistryEntry {
    name: ToolName,
    exposure: ToolExposure,
    spec: Option<ToolSpec>,
    search_info: Option<ToolSearchInfo>,
}

impl ToolRegistryEntry for TestRegistryEntry {
    fn tool_name(&self) -> ToolName {
        self.name.clone()
    }

    fn spec(&self) -> Option<ToolSpec> {
        self.spec.clone()
    }

    fn exposure(&self) -> ToolExposure {
        self.exposure
    }

    fn search_info(&self) -> Option<ToolSearchInfo> {
        self.search_info.clone()
    }
}

fn function_tool(name: &str, description: &str) -> ResponsesApiTool {
    ResponsesApiTool {
        name: name.to_string(),
        description: description.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            BTreeMap::new(),
            /*required*/ None,
            /*additional_properties*/ None,
        ),
        output_schema: None,
    }
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

fn tools_config_with_agent_tool_patterns(patterns: Vec<String>) -> ToolsConfig {
    let features = Features::with_defaults();
    let model_info = model_info();
    let available_models = Vec::new();
    ToolsConfig::new(&crate::ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Live),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    })
    .with_agent_tool_patterns(Some(patterns))
}

fn model_info() -> ModelInfo {
    serde_json::from_value(json!({
        "slug": "gpt-5-codex",
        "display_name": "GPT-5 Codex",
        "description": null,
        "supported_reasoning_levels": [],
        "shell_type": "shell_command",
        "visibility": "list",
        "supported_in_api": true,
        "priority": 1,
        "availability_nux": null,
        "upgrade": null,
        "base_instructions": "base",
        "model_messages": null,
        "supports_reasoning_summaries": false,
        "default_reasoning_summary": "auto",
        "support_verbosity": false,
        "default_verbosity": null,
        "apply_patch_tool_type": "freeform",
        "truncation_policy": {
            "mode": "bytes",
            "limit": 10000
        },
        "supports_parallel_tool_calls": false,
        "supports_image_detail_original": false,
        "context_window": null,
        "auto_compact_token_limit": null,
        "effective_context_window_percent": 95,
        "experimental_supported_tools": [],
        "input_modalities": ["text", "image"],
        "supports_search_tool": false
    }))
    .expect("deserialize test model")
}
