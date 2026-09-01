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
const FOLLOWUP_USAGE_GUIDANCE: &str = "Use this to send work, corrections, extra context, status requests, or decisions to another agent. If a parent or another agent asks you to report status, progress, interim findings, blockers, or decision needs to them, call this tool targeting that agent; do not answer only in your current thread. A normal assistant response only advances or completes your current thread and does not deliver a typed inter-agent update to the requested target. Examples: report progress to your parent; send a blocker to the PM; ask a reviewer to re-review; pass new requirements to a worker.";

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
    let properties = followup_task_properties(
        "External or native agent id/canonical task name to message.",
        "Message text to send through the shared agent bus.",
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "followup_external_task".to_string(),
        description: format!(
            "Send a follow-up message to an existing target agent from the external-agent collaboration surface. This uses the same backend agent bus as native followup_task while keeping the model-visible external protocol separate. External agents must use this external tool surface, not internal Morpheus followup_task. {FOLLOWUP_USAGE_GUIDANCE}"
        ),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["target".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

fn followup_task_properties(
    target_description: &str,
    message_description: &str,
) -> BTreeMap<String, JsonSchema> {
    BTreeMap::from([
        (
            "target".to_string(),
            JsonSchema::string(Some(target_description.to_string())),
        ),
        (
            "message".to_string(),
            JsonSchema::string(Some(format!(
                "{message_description} Do not write image placeholders such as `[image:image-1]` or `<image attachment_id=image-1>` here; image references must use `content: [{{\"type\":\"image_ref\",\"attachment_id\":\"image-1\"}}]`."
            ))),
        ),
        (
            "content".to_string(),
            JsonSchema::array(
                JsonSchema::object(
                    BTreeMap::from([
                        (
                            "type".to_string(),
                            JsonSchema::string(Some(
                                "`text` for text parts or `image_ref` for image references."
                                    .to_string(),
                            )),
                        ),
                        (
                            "text".to_string(),
                            JsonSchema::string(Some("Text for a `text` content part.".to_string())),
                        ),
                        (
                            "attachment_id".to_string(),
                            JsonSchema::string(Some(
                                "Model-visible attachment id for an image visible in the parent thread, for example `image-1`."
                                    .to_string(),
                            )),
                        ),
                    ]),
                    Some(vec!["type".to_string()]),
                    Some(false.into()),
                ),
                Some(
                    "Structured followup content. Use text parts and image_ref parts; image_ref requires an attachment_id that is visible in the parent thread. Do not put image placeholders inside text parts; use a separate image_ref part instead."
                        .to_string(),
                ),
            ),
        ),
    ])
}

pub fn create_poll_external_event_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "poll_external_event".to_string(),
        description: "Wait for the next new thread input that reaches the external-agent bus, such as user input, child completion or other inter-agent updates, command output or exit notifications, or other queued model-consumable input. This returns wake or timeout metadata plus a best-effort source hint and typed event payload when one is available.".to_string(),
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

pub fn create_read_external_agent_tool() -> ToolSpec {
    let ToolSpec::Function(mut tool) = create_read_agent_tool() else {
        unreachable!("read_agent is a function tool");
    };
    tool.name = "read_external_agent".to_string();
    tool.description = "Read details for one agent visible to the external-agent collaboration surface, including last task and final result text when available.".to_string();
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
    let properties = followup_task_properties(
        "Agent id or canonical task name to message (from spawn_agent).",
        "Message text to send to the target agent.",
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "followup_task".to_string(),
        description: format!(
            "Send a follow-up message to an existing non-root target agent. {FOLLOWUP_USAGE_GUIDANCE}"
        ),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["target".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

pub fn create_poll_event_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "poll_event".to_string(),
        description: "Wait for the next new thread input that reaches the active turn runtime, such as user input, child completion or other inter-agent updates, command output or exit notifications, or other queued model-consumable input. This returns wake or timeout metadata plus a best-effort source hint. When a typed event is available, command output/exit wakeups include the concrete command notification payload.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(BTreeMap::new(), Some(Vec::new()), Some(false.into())),
        output_schema: Some(poll_event_output_schema()),
    })
}

pub fn create_agent_role_load_tool() -> ToolSpec {
    let properties = BTreeMap::from([(
        "agent_type".to_string(),
        JsonSchema::string(Some(
            "Agent role/type to load into the current Morpheus native thread. Must be one of the configured or built-in agent types visible to this session.".to_string(),
        )),
    )]);

    ToolSpec::Function(ResponsesApiTool {
        name: "agent_role_load".to_string(),
        description: "Load an agent role into the current Morpheus native thread. The requested role is resolved from the session's configured and built-in agent types, then applied to the live runtime and persisted thread metadata for subsequent turns, thread reads, lists, resumes, and compaction.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["agent_type".to_string()]),
            Some(false.into()),
        ),
        output_schema: Some(agent_role_load_output_schema()),
    })
}

fn agent_role_load_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "agentRole": {
                "type": "string",
                "description": "The loaded agent role."
            },
            "effective": {
                "type": "string",
                "enum": ["next_turn"],
                "description": "When the loaded role becomes part of model-visible turn context."
            },
            "model": {
                "type": "string",
                "description": "The effective session model after applying the role."
            },
            "reasoningEffort": {
                "type": ["string", "null"],
                "description": "The effective reasoning effort after applying the role."
            }
        },
        "required": ["agentRole", "effective", "model", "reasoningEffort"],
        "additionalProperties": false
    })
}

fn poll_event_output_schema() -> serde_json::Value {
    let poll_event_item_schema = poll_event_item_output_schema();
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
            "event": {
                "anyOf": [
                    poll_event_item_schema.clone(),
                    { "type": "null" }
                ],
                "description": "Typed payload for the pending event when available. Command output/exit wakeups include the concrete command notification payload here."
            },
            "events": {
                "type": "array",
                "items": poll_event_item_schema,
                "description": "All typed pending event payloads currently visible to the active turn runtime, including command output/exit payloads."
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

fn poll_event_item_output_schema() -> serde_json::Value {
    json!({
        "anyOf": [
            {
                "type": "object",
                "properties": {
                    "type": {
                        "type": "string",
                        "const": "inter_agent_communication"
                    },
                    "communication": {
                        "type": "object",
                        "description": "Typed inter-agent communication payload. Child completion events carry the child final status in communication.status and the completion text in communication.content."
                    }
                },
                "required": ["type", "communication"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "type": {
                        "type": "string",
                        "const": "command_execution_notification"
                    },
                    "commandItemId": {
                        "type": "string",
                        "description": "Command execution item id associated with a command output or exit notification."
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["output", "exit"],
                        "description": "Whether the command notification reports new output or process exit."
                    },
                    "message": {
                        "type": "string",
                        "description": "Human-readable command notification message."
                    },
                    "output": {
                        "type": ["string", "null"],
                        "description": "Bounded command output included with the notification when available."
                    },
                    "exitCode": {
                        "type": ["number", "null"],
                        "description": "Command process exit code for exit notifications when available."
                    },
                    "createdAtMs": {
                        "type": "number",
                        "description": "Notification creation timestamp in milliseconds."
                    }
                },
                "required": [
                    "type",
                    "commandItemId",
                    "kind",
                    "message",
                    "createdAtMs"
                ],
                "additionalProperties": false
            }
        ]
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

pub fn create_read_agent_tool() -> ToolSpec {
    let properties = BTreeMap::from([(
        "target".to_string(),
        JsonSchema::string(Some(
            "Agent id or canonical task name/path to inspect, usually chosen from list_agents."
                .to_string(),
        )),
    )]);

    ToolSpec::Function(ResponsesApiTool {
        name: "read_agent".to_string(),
        description: "Read details for one agent visible in the current root thread tree, including its last task message and final result text when available. Use after list_agents when you need more than lightweight directory metadata."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(properties, Some(vec!["target".to_string()]), Some(false.into())),
        output_schema: Some(read_agent_output_schema()),
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
    lifecycle_status_output_schema_with_completed_message(true)
}

fn lightweight_lifecycle_status_output_schema() -> Value {
    lifecycle_status_output_schema_with_completed_message(false)
}

fn lifecycle_status_output_schema_with_completed_message(include_completed_message: bool) -> Value {
    let completed_schema = if include_completed_message {
        json!({
            "type": "object",
            "properties": {
                "type": { "type": "string", "enum": ["completed"] },
                "last_agent_message": { "type": ["string", "null"] }
            },
            "required": ["type"],
            "additionalProperties": false
        })
    } else {
        json!({
            "type": "object",
            "properties": {
                "type": { "type": "string", "enum": ["completed"] }
            },
            "required": ["type"],
            "additionalProperties": false
        })
    };

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
                            completed_schema,
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
                        "agentName": {
                            "type": "string",
                            "description": "Canonical task name for the agent when available, otherwise the agent id."
                        },
                        "agentNickname": {
                            "type": ["string", "null"],
                            "description": "User-facing nickname for the agent when available."
                        },
                        "agentRole": {
                            "type": ["string", "null"],
                            "description": "Configured role/type for the agent when available."
                        },
                        "lifecycleStatus": {
                            "description": "Last known lifecycle status of the agent thread.",
                            "allOf": [lightweight_lifecycle_status_output_schema()]
                        }
                    },
                    "required": ["agentName", "agentNickname", "agentRole", "lifecycleStatus"],
                    "additionalProperties": false
                },
                "description": "Live agents visible in the current root thread tree."
            }
        },
        "required": ["agents"],
        "additionalProperties": false
    })
}

fn read_agent_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "agent": {
                "type": "object",
                "properties": {
                    "agentName": {
                        "type": "string",
                        "description": "Canonical task name for the agent when available, otherwise the agent id."
                    },
                    "agentNickname": {
                        "type": ["string", "null"],
                        "description": "User-facing nickname for the agent when available."
                    },
                    "agentRole": {
                        "type": ["string", "null"],
                        "description": "Configured role/type for the agent when available."
                    },
                    "lifecycleStatus": {
                        "description": "Full last known lifecycle status of the agent thread.",
                        "allOf": [lifecycle_status_output_schema()]
                    },
                    "lastTaskMessage": {
                        "type": ["string", "null"],
                        "description": "Most recent user or inter-agent instruction received by the agent, when available."
                    }
                },
                "required": ["agentName", "agentNickname", "agentRole", "lifecycleStatus", "lastTaskMessage"],
                "additionalProperties": false
            }
        },
        "required": ["agent"],
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
