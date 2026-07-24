use crate::JsonSchema;
use crate::ResponsesApiTool;
use crate::ToolSpec;
use protocol::openai_models::ModelPreset;
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;

const SPAWN_AGENT_INHERITED_MODEL_GUIDANCE: &str = "Spawned agents inherit your current model by default. Omit `model` to use that preferred default; set `model` only when an explicit override is needed.";
const SPAWN_AGENT_MODEL_OVERRIDE_DESCRIPTION: &str = "Optional model override for the new agent. Leave unset to inherit the same model as the parent, which is the preferred default. Only set this when the user explicitly asks for a different model or the task clearly requires one.";
const SPAWN_AGENT_SERVICE_TIER_OVERRIDE_DESCRIPTION: &str = "Optional service tier override for the new agent. Leave unset unless the user explicitly asks for one.";
const SPAWN_AGENT_CWD_AGENT_TYPE_DESCRIPTION: &str = "When `cwd` is set, agent types from that cwd or its repository may be used even if they are not listed in your current context.";

#[derive(Debug, Clone, Default)]
pub struct SpawnAgentToolOptions {
    pub available_models: Vec<ModelPreset>,
    pub agent_type_description: String,
    pub hide_agent_type_model_reasoning: bool,
    pub include_usage_hint: bool,
    pub usage_hint_text: Option<String>,
    pub max_concurrent_threads_per_session: Option<usize>,
}

pub fn create_spawn_agent_tool_v2(options: SpawnAgentToolOptions) -> ToolSpec {
    let available_models_description = (!options.hide_agent_type_model_reasoning)
        .then(|| spawn_agent_models_description(&options.available_models));
    let mut properties = spawn_agent_common_properties_v2(&options.agent_type_description);
    if options.hide_agent_type_model_reasoning {
        hide_spawn_agent_metadata_options(&mut properties);
    }
    properties.insert(
        "task_name".to_string(),
        JsonSchema::string(Some(
            "Task name for the new agent. Use lowercase letters, digits, and underscores."
                .to_string(),
        )),
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "spawn_agent".to_string(),
        description: spawn_agent_tool_description_v2(
            available_models_description.as_deref(),
            options.include_usage_hint,
            options.usage_hint_text,
            options.max_concurrent_threads_per_session,
        ),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["task_name".to_string(), "message".to_string()]),
            Some(false.into()),
        ),
        output_schema: Some(spawn_agent_output_schema_v2(
            options.hide_agent_type_model_reasoning,
        )),
    })
}

pub fn create_spawn_external_agent_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "task_name".to_string(),
            JsonSchema::string(Some(
                "Task name for the external agent. Use lowercase letters, digits, and underscores."
                    .to_string(),
            )),
        ),
        (
            "provider".to_string(),
            JsonSchema::string_enum(
                vec![json!("claude_cli"), json!("opencode"), json!("codex_cli")],
                Some(
                    "External code agent provider to launch. Current external session transport support includes claude_cli stream-json, opencode HTTP sessions, and codex_cli app-server stdio sessions."
                        .to_string(),
                ),
            ),
        ),
        (
            "cwd".to_string(),
            JsonSchema::string(Some(
                "Working directory for the external CLI agent. Required.".to_string(),
            )),
        ),
        (
            "message".to_string(),
            JsonSchema::string(Some(
                "Initial plain-text task for the external code agent.".to_string(),
            )),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "spawn_external_agent".to_string(),
        description: "Spawn an external code-agent CLI as a child in the current agent tree. External agents use the external-agent JSON tool protocol for collaboration; use spawn_agent only for Morpheus native agents.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec![
                "task_name".to_string(),
                "provider".to_string(),
                "cwd".to_string(),
                "message".to_string(),
            ]),
            Some(false.into()),
        ),
        output_schema: Some(spawn_agent_output_schema_v2(false)),
    })
}

pub fn create_followup_external_task_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "target".to_string(),
            JsonSchema::string(Some(
                "External or native agent id/canonical task name to message.".to_string(),
            )),
        ),
        (
            "message".to_string(),
            JsonSchema::string(Some(
                "Message text to send through the shared agent bus.".to_string(),
            )),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "followup_external_task".to_string(),
        description: "Send a follow-up message from the external-agent collaboration surface. This uses the same backend agent bus as native followup_task while keeping the model-visible external protocol separate.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(properties, Some(vec!["target".to_string(), "message".to_string()]), Some(false.into())),
        output_schema: None,
    })
}

pub fn create_poll_external_event_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "poll_external_event".to_string(),
        description: "Wait for the next new thread input that reaches the external-agent bus, such as user input, child completion or other inter-agent updates, command output or exit notifications, or other queued model-consumable input. This returns only wake or timeout metadata plus a best-effort source hint, not the event payload.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(BTreeMap::new(), Some(Vec::new()), Some(false.into())),
        output_schema: Some(poll_event_output_schema()),
    })
}

pub fn create_list_external_agents_tool() -> ToolSpec {
    let ToolSpec::Function(mut tool) = create_list_agents_tool() else {
        unreachable!("list_agents is a function tool");
    };
    tool.name = "list_external_agents".to_string();
    tool.description = "List agents visible to the external-agent collaboration surface, including native and external agents with lifecycle/provider metadata.".to_string();
    ToolSpec::Function(tool)
}

pub fn create_close_external_agent_tool() -> ToolSpec {
    let ToolSpec::Function(mut tool) = create_close_agent_tool_v2() else {
        unreachable!("close_agent is a function tool");
    };
    tool.name = "close_external_agent".to_string();
    tool.description = "Close an external-agent collaboration target and any open descendants when no longer needed.".to_string();
    ToolSpec::Function(tool)
}

pub fn create_followup_task_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "target".to_string(),
            JsonSchema::string(Some(
                "Agent id or canonical task name to message (from spawn_agent).".to_string(),
            )),
        ),
        (
            "message".to_string(),
            JsonSchema::string(Some(
                "Message text to send to the target agent.".to_string(),
            )),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "followup_task".to_string(),
        description: "Send a message to an existing non-root target agent and trigger a turn in that target. If the target is currently mid-turn, the message is queued and will be used to start the target's next turn, after the current turn completes."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(properties, Some(vec!["target".to_string(), "message".to_string()]), Some(false.into())),
        output_schema: None,
    })
}

pub fn create_poll_event_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "poll_event".to_string(),
        description: "Wait for the next new thread input that reaches the active turn runtime, such as user input, child completion or other inter-agent updates, command output or exit notifications, or other queued model-consumable input. This returns only wake or timeout metadata plus a best-effort source hint, not the event payload.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(BTreeMap::new(), Some(Vec::new()), Some(false.into())),
        output_schema: Some(poll_event_output_schema()),
    })
}

fn poll_event_output_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "timedOut": {
                "type": "boolean",
                "description": "Whether the wait window elapsed without a new thread input."
            },
            "sourceHint": {
                "type": ["string", "null"],
                "description": "Best-effort hint for the source that woke the wait, such as user_input, child_completion, inter_agent, command_output, command_exit, queued_input, or async_input."
            },
            "waitedMs": {
                "type": "number",
                "description": "Elapsed wall-clock wait time in milliseconds."
            },
            "initialTimeoutMs": {
                "type": "number",
                "description": "Configured initial wait window in milliseconds."
            },
            "currentTimeoutMs": {
                "type": "number",
                "description": "Current backoff-adjusted wait window in milliseconds."
            },
            "hardCapTimeoutMs": {
                "type": "number",
                "description": "Maximum backoff-adjusted wait window in milliseconds."
            }
        },
        "required": [
            "timedOut",
            "sourceHint",
            "waitedMs",
            "initialTimeoutMs",
            "currentTimeoutMs",
            "hardCapTimeoutMs"
        ],
        "additionalProperties": false
    })
}

pub fn create_list_agents_tool() -> ToolSpec {
    let properties = BTreeMap::from([(
        "path_prefix".to_string(),
        JsonSchema::string(Some(
            "Optional task-path prefix (not ending with trailing slash). Accepts the same relative or absolute task-path syntax."
                .to_string(),
        )),
    )]);

    ToolSpec::Function(ResponsesApiTool {
        name: "list_agents".to_string(),
        description: "List registered agents in the current root thread tree whose live status is still available, including completed agents that are still known to the runtime. Optionally filter by task-path prefix."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(properties, /*required*/ None, Some(false.into())),
        output_schema: Some(list_agents_output_schema()),
    })
}

pub fn create_close_agent_tool_v2() -> ToolSpec {
    let properties = BTreeMap::from([(
        "target".to_string(),
        JsonSchema::string(Some(
            "Agent id or canonical task name to close (from spawn_agent).".to_string(),
        )),
    )]);

    ToolSpec::Function(ResponsesApiTool {
        name: "close_agent".to_string(),
        description: "Close an agent and any open descendants when they are no longer needed, and return the target agent's previous status before shutdown was requested. Don't keep agents open for too long if they are not needed anymore.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(properties, Some(vec!["target".to_string()]), Some(false.into())),
        output_schema: Some(close_agent_output_schema()),
    })
}

fn agent_status_output_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "string",
                "enum": ["pending_init", "running", "interrupted", "shutdown", "not_found"]
            },
            {
                "type": "object",
                "properties": {
                    "completed": {
                        "type": ["string", "null"]
                    }
                },
                "required": ["completed"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "errored": {
                        "type": "string"
                    }
                },
                "required": ["errored"],
                "additionalProperties": false
            }
        ]
    })
}

fn lifecycle_status_output_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "enum": ["notLoaded"] }
                },
                "required": ["type"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "enum": ["initializing"] }
                },
                "required": ["type"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "enum": ["active"] },
                    "activeFlags": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "enum": ["running", "waitingOnApproval", "waitingOnUserInput"]
                        }
                    }
                },
                "required": ["type", "activeFlags"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "enum": ["waiting"] },
                    "reason": {
                        "type": "string",
                        "enum": ["child", "command", "eventSubscription"]
                    }
                },
                "required": ["type", "reason"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "enum": ["final"] },
                    "result": {
                        "oneOf": [
                            {
                                "type": "object",
                                "properties": {
                                    "type": { "type": "string", "enum": ["completed"] },
                                    "lastAgentMessage": { "type": ["string", "null"] }
                                },
                                "required": ["type"],
                                "additionalProperties": false
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "type": { "type": "string", "enum": ["errored"] },
                                    "message": { "type": ["string", "null"] }
                                },
                                "required": ["type"],
                                "additionalProperties": false
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "type": { "type": "string", "enum": ["interrupted"] }
                                },
                                "required": ["type"],
                                "additionalProperties": false
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "type": { "type": "string", "enum": ["shutdown"] }
                                },
                                "required": ["type"],
                                "additionalProperties": false
                            }
                        ]
                    }
                },
                "required": ["type", "result"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "enum": ["systemError"] },
                    "message": { "type": ["string", "null"] }
                },
                "required": ["type"],
                "additionalProperties": false
            }
        ]
    })
}

fn spawn_agent_output_schema_v2(hide_agent_metadata: bool) -> Value {
    if hide_agent_metadata {
        return json!({
            "type": "object",
            "properties": {
                "task_name": {
                    "type": "string",
                    "description": "Canonical task name for the spawned agent."
                }
            },
            "required": ["task_name"],
            "additionalProperties": false
        });
    }

    json!({
        "type": "object",
        "properties": {
            "task_name": {
                "type": "string",
                "description": "Canonical task name for the spawned agent."
            },
            "nickname": {
                "type": ["string", "null"],
                "description": "User-facing nickname for the spawned agent when available."
            }
        },
        "required": ["task_name", "nickname"],
        "additionalProperties": false
    })
}

fn list_agents_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "agents": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "agent_name": {
                            "type": "string",
                            "description": "Canonical task name for the agent when available, otherwise the agent id."
                        },
                        "lifecycle_status": {
                            "description": "Last known lifecycle status of the agent thread.",
                            "allOf": [lifecycle_status_output_schema()]
                        },
                        "last_task_message": {
                            "type": ["string", "null"],
                            "description": "Most recent user or inter-agent instruction received by the agent, when available."
                        }
                    },
                    "required": ["agent_name", "lifecycle_status", "last_task_message"],
                    "additionalProperties": false
                },
                "description": "Live agents visible in the current root thread tree."
            }
        },
        "required": ["agents"],
        "additionalProperties": false
    })
}

fn close_agent_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "previous_status": {
                "description": "The agent status observed before shutdown was requested.",
                "allOf": [agent_status_output_schema()]
            }
        },
        "required": ["previous_status"],
        "additionalProperties": false
    })
}

fn spawn_agent_common_properties_v2(agent_type_description: &str) -> BTreeMap<String, JsonSchema> {
    BTreeMap::from([
        (
            "message".to_string(),
            JsonSchema::string(Some("Initial plain-text task for the new agent.".to_string())),
        ),
        (
            "agent_type".to_string(),
            JsonSchema::string(Some(spawn_agent_type_description(agent_type_description))),
        ),
        (
            "cwd".to_string(),
            JsonSchema::string(Some(
                "Optional working directory for the spawned agent. Defaults to the parent turn cwd."
                    .to_string(),
            )),
        ),
        (
            "fork_turns".to_string(),
            JsonSchema::string(Some(
                "Optional number of turns to fork. Defaults to `all`. Use `none`, `all`, or a positive integer string such as `3` to fork only the most recent turns."
                    .to_string(),
            )),
        ),
        (
            "model".to_string(),
            JsonSchema::string(Some(
                SPAWN_AGENT_MODEL_OVERRIDE_DESCRIPTION.to_string(),
            )),
        ),
        (
            "reasoning_effort".to_string(),
            JsonSchema::string(Some(
                "Optional reasoning effort override for the new agent. Replaces the inherited reasoning effort."
                    .to_string(),
            )),
        ),
        (
            "service_tier".to_string(),
            JsonSchema::string(Some(
                SPAWN_AGENT_SERVICE_TIER_OVERRIDE_DESCRIPTION.to_string(),
            )),
        ),
    ])
}

fn spawn_agent_type_description(agent_type_description: &str) -> String {
    format!("{agent_type_description}\n\n{SPAWN_AGENT_CWD_AGENT_TYPE_DESCRIPTION}")
}

fn hide_spawn_agent_metadata_options(properties: &mut BTreeMap<String, JsonSchema>) {
    properties.remove("agent_type");
    properties.remove("model");
    properties.remove("reasoning_effort");
    properties.remove("service_tier");
}

fn spawn_agent_tool_description_v2(
    available_models_description: Option<&str>,
    include_usage_hint: bool,
    usage_hint_text: Option<String>,
    max_concurrent_threads_per_session: Option<usize>,
) -> String {
    let agent_role_guidance = available_models_description.unwrap_or_default();
    let concurrency_guidance = max_concurrent_threads_per_session
        .map(|limit| {
            format!(
                "This session is configured with `max_concurrent_threads_per_session = {limit}` for concurrently open agent threads."
            )
        })
        .unwrap_or_default();

    let tool_description = format!(
        r#"
        {agent_role_guidance}
        Spawns an agent to work on the specified task. If your current task is `/root/task1` and you spawn_agent with task_name "task_3" the agent will have canonical task name `/root/task1/task_3`.
You are then able to refer to this agent as `task_3` or `/root/task1/task_3` interchangeably. However an agent `/root/task2/task_3` would only be able to communicate with this agent via its canonical name `/root/task1/task_3`.
The spawned agent will have the same tools as you and the ability to spawn its own subagents.
{SPAWN_AGENT_INHERITED_MODEL_GUIDANCE}
It will be able to send you and other running agents messages, and its final answer will be provided to you when it finishes.
Sub-agent completion is delivered automatically; do not poll or use other tools just to wait for it.
The new agent's canonical task name will be provided to it along with the message.
{concurrency_guidance}"#
    );

    if !include_usage_hint {
        return tool_description;
    }
    if let Some(usage_hint_text) = usage_hint_text {
        return format!(
            r#"
        {tool_description}
{usage_hint_text}"#
        );
    }
    tool_description
}

fn spawn_agent_models_description(models: &[ModelPreset]) -> String {
    let visible_models: Vec<&ModelPreset> =
        models.iter().filter(|model| model.show_in_picker).collect();
    if visible_models.is_empty() {
        return "No picker-visible model overrides are currently loaded.".to_string();
    }

    let model_descriptions = visible_models
        .into_iter()
        .map(|model| {
            let efforts = model
                .supported_reasoning_efforts
                .iter()
                .map(|preset| format!("{} ({})", preset.effort, preset.description))
                .collect::<Vec<_>>()
                .join(", ");
            let service_tiers = if model.service_tiers.is_empty() {
                "none".to_string()
            } else {
                model
                    .service_tiers
                    .iter()
                    .map(|tier| format!("{} ({}: {})", tier.id, tier.name, tier.description))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            format!(
                "- {} (`{}`): {} Default reasoning effort: {}. Supported reasoning efforts: {}. Supported service tiers: {}.",
                model.display_name,
                model.model,
                model.description,
                model.default_reasoning_effort,
                efforts,
                service_tiers
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Available model overrides (optional; inherited parent model is preferred):\n{model_descriptions}"
    )
}

#[cfg(test)]
#[path = "multi_agents_tests.rs"]
mod tests;
