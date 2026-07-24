use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;

use crate::WorkflowRuntimeError;
use crate::WorkflowRuntimeRequest;

#[derive(Debug, Deserialize)]
pub struct WorkflowDescribeArgs {
    pub workflow: String,
}

impl WorkflowDescribeArgs {
    pub fn workflow(&self) -> Result<&str, String> {
        required_non_empty_arg(&self.workflow, "workflow")
    }
}

#[derive(Debug, Deserialize)]
pub struct WorkflowStartArgs {
    pub workflow: String,
    #[serde(default)]
    pub inputs: Option<Value>,
}

impl WorkflowStartArgs {
    pub fn workflow(&self) -> Result<&str, String> {
        required_non_empty_arg(&self.workflow, "workflow")
    }
}

#[derive(Debug, Deserialize)]
pub struct WorkflowStatusArgs {
    pub run_id: String,
}

impl WorkflowStatusArgs {
    pub fn run_id(&self) -> Result<&str, String> {
        required_non_empty_arg(&self.run_id, "run_id")
    }
}

#[derive(Debug, Deserialize)]
pub struct WorkflowResumeArgs {
    pub run_id: String,
    #[serde(default)]
    pub inputs: Option<Value>,
}

impl WorkflowResumeArgs {
    pub fn run_id(&self) -> Result<&str, String> {
        required_non_empty_arg(&self.run_id, "run_id")
    }
}

#[derive(Debug, Deserialize)]
pub struct WorkflowAbortArgs {
    pub run_id: String,
    #[serde(default)]
    pub reason: Option<String>,
}

impl WorkflowAbortArgs {
    pub fn run_id(&self) -> Result<&str, String> {
        required_non_empty_arg(&self.run_id, "run_id")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowSpawnAgentToolCall {
    pub agent_id: String,
    pub options: Value,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowFollowupTaskToolCall {
    pub target: String,
    pub message: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowWaitAgentToolCall {
    pub target: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowPollEventToolCall {
    pub arguments: Value,
}

pub fn workflow_spawn_agent_tool_call(
    request: &WorkflowRuntimeRequest,
) -> Result<WorkflowSpawnAgentToolCall, WorkflowRuntimeError> {
    let agent_id = required_runtime_string(&request.params, "id")?;
    let options = request
        .params
        .get("options")
        .cloned()
        .unwrap_or(Value::Null);
    let message = required_runtime_string(&options, "message")?;
    let mut arguments = json!({
        "message": message,
        "task_name": workflow_agent_task_name(&request.run_id, &agent_id),
    });
    copy_option_string(
        &mut arguments,
        "agent_type",
        &options,
        &["type", "agent_type"],
    );
    copy_option_string(&mut arguments, "cwd", &options, &["cwd"]);
    copy_option_string(&mut arguments, "model", &options, &["model"]);
    copy_option_string(
        &mut arguments,
        "reasoning_effort",
        &options,
        &["reasoningEffort", "reasoning_effort"],
    );
    copy_option_string(
        &mut arguments,
        "service_tier",
        &options,
        &["serviceTier", "service_tier"],
    );
    copy_option_string(
        &mut arguments,
        "fork_turns",
        &options,
        &["forkTurns", "fork_turns"],
    );

    Ok(WorkflowSpawnAgentToolCall {
        agent_id,
        options,
        arguments,
    })
}

pub fn workflow_followup_task_tool_call(
    request: &WorkflowRuntimeRequest,
) -> Result<WorkflowFollowupTaskToolCall, WorkflowRuntimeError> {
    let target = required_runtime_string(&request.params, "target")?;
    let message = required_runtime_string(&request.params, "message")?;
    let arguments = json!({
        "target": target,
        "message": message,
    });
    Ok(WorkflowFollowupTaskToolCall {
        target,
        message,
        arguments,
    })
}

pub fn workflow_wait_agent_tool_call(
    request: &WorkflowRuntimeRequest,
) -> Result<WorkflowWaitAgentToolCall, WorkflowRuntimeError> {
    let target = required_runtime_string(&request.params, "target")?;
    let arguments = json!({ "target": target });
    Ok(WorkflowWaitAgentToolCall { target, arguments })
}

pub fn workflow_poll_event_tool_call(
    request: &WorkflowRuntimeRequest,
) -> Result<WorkflowPollEventToolCall, WorkflowRuntimeError> {
    reject_runtime_field(&request.params, "id")?;
    reject_runtime_field(&request.params, "target")?;
    Ok(WorkflowPollEventToolCall {
        arguments: json!({}),
    })
}

pub fn workflow_tool_call_id(request: &WorkflowRuntimeRequest, tool_name: &str) -> String {
    format!("workflow:{}:{}:{tool_name}", request.run_id, request.rpc_id)
}

pub fn workflow_agent_task_name(run_id: &str, stage_id: &str) -> String {
    format!(
        "workflow_{}_{}_{}",
        path_safe_segment(run_id),
        path_safe_segment(stage_id),
        stable_hex_hash(stage_id)
    )
}

pub fn workflow_tool_output_json<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(value)
}

fn required_non_empty_arg<'a>(value: &'a str, field: &str) -> Result<&'a str, String> {
    let value = value.trim();
    if value.is_empty() {
        Err(format!("{field} must not be empty"))
    } else {
        Ok(value)
    }
}

fn required_runtime_string(value: &Value, field: &str) -> Result<String, WorkflowRuntimeError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| WorkflowRuntimeError::invalid_request(format!("missing `{field}`")))
}

fn reject_runtime_field(value: &Value, field: &str) -> Result<(), WorkflowRuntimeError> {
    if value.get(field).is_some() {
        Err(WorkflowRuntimeError::invalid_request(format!(
            "`{field}` is not supported for event.poll"
        )))
    } else {
        Ok(())
    }
}

fn copy_option_string(target: &mut Value, target_field: &str, source: &Value, fields: &[&str]) {
    let Some(value) = fields
        .iter()
        .find_map(|field| source.get(*field).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };
    target[target_field] = Value::String(value.to_string());
}

fn path_safe_segment(value: &str) -> String {
    let mut output = String::new();
    let mut previous_was_underscore = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        let safe = if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            previous_was_underscore = false;
            Some(ch)
        } else if !previous_was_underscore {
            previous_was_underscore = true;
            Some('_')
        } else {
            None
        };
        if let Some(ch) = safe {
            output.push(ch);
        }
    }

    let trimmed = output.trim_matches('_');
    if trimmed.is_empty() {
        "agent".to_string()
    } else {
        trimmed.to_string()
    }
}

fn stable_hex_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_agent_task_name_uses_runtime_owned_safe_name() {
        assert_eq!(
            workflow_agent_task_name("wf_1781667698_3", "review/fix Stage"),
            "workflow_wf_1781667698_3_review_fix_stage_6a50e610f22115e3"
        );
        assert_eq!(
            workflow_agent_task_name("wf_1", "../root"),
            "workflow_wf_1_root_fcf22ea2feead752"
        );
        assert_eq!(
            workflow_agent_task_name("wf_1", "!!!"),
            "workflow_wf_1_agent_bbe43c17ca866be2"
        );
    }

    #[test]
    fn workflow_agent_task_name_keeps_colliding_slugs_distinct() {
        assert_ne!(
            workflow_agent_task_name("wf_1", "review/fix"),
            workflow_agent_task_name("wf_1", "review_fix")
        );
        assert_ne!(
            workflow_agent_task_name("wf_1", "review fix"),
            workflow_agent_task_name("wf_1", "review_fix")
        );
    }

    #[test]
    fn workflow_spawn_agent_tool_call_maps_sdk_options_to_spawn_args() {
        let request = WorkflowRuntimeRequest {
            run_id: "wf_1".to_string(),
            workflow_id: "feature-dev".to_string(),
            rpc_id: 7,
            method: "agent.spawn".to_string(),
            params: json!({
                "id": "review/fix",
                "options": {
                    "message": "review this",
                    "type": "code-review",
                    "cwd": "/tmp/project",
                    "reasoningEffort": "high",
                    "serviceTier": "priority",
                    "forkTurns": "none"
                }
            }),
        };

        let tool_call = workflow_spawn_agent_tool_call(&request).expect("spawn request should map");

        assert_eq!(tool_call.agent_id, "review/fix");
        assert_eq!(
            tool_call.arguments,
            json!({
                "message": "review this",
                "task_name": "workflow_wf_1_review_fix_0cd781fab25429fd",
                "agent_type": "code-review",
                "cwd": "/tmp/project",
                "reasoning_effort": "high",
                "service_tier": "priority",
                "fork_turns": "none"
            })
        );
    }

    #[test]
    fn workflow_followup_and_wait_tool_calls_preserve_target() {
        let followup = WorkflowRuntimeRequest {
            run_id: "wf_1".to_string(),
            workflow_id: "feature-dev".to_string(),
            rpc_id: 8,
            method: "agent.followup".to_string(),
            params: json!({
                "target": "/root/agent",
                "message": "continue"
            }),
        };
        assert_eq!(
            workflow_followup_task_tool_call(&followup)
                .expect("followup should map")
                .arguments,
            json!({
                "target": "/root/agent",
                "message": "continue"
            })
        );

        let wait = WorkflowRuntimeRequest {
            method: "agent.wait".to_string(),
            params: json!({ "target": "/root/agent" }),
            ..followup
        };
        assert_eq!(
            workflow_wait_agent_tool_call(&wait)
                .expect("wait should map")
                .arguments,
            json!({ "target": "/root/agent" })
        );
    }

    #[test]
    fn workflow_poll_event_tool_call_has_no_target() {
        let request = WorkflowRuntimeRequest {
            run_id: "wf_1".to_string(),
            workflow_id: "feature-dev".to_string(),
            rpc_id: 9,
            method: "event.poll".to_string(),
            params: json!({}),
        };

        assert_eq!(
            workflow_poll_event_tool_call(&request)
                .expect("poll event should map")
                .arguments,
            json!({})
        );
    }

    #[test]
    fn workflow_poll_event_tool_call_rejects_agent_target() {
        let request = WorkflowRuntimeRequest {
            run_id: "wf_1".to_string(),
            workflow_id: "feature-dev".to_string(),
            rpc_id: 9,
            method: "event.poll".to_string(),
            params: json!({ "target": "/root/agent" }),
        };

        assert!(workflow_poll_event_tool_call(&request).is_err());
    }

    #[test]
    fn workflow_poll_event_tool_call_rejects_agent_id() {
        let request = WorkflowRuntimeRequest {
            run_id: "wf_1".to_string(),
            workflow_id: "feature-dev".to_string(),
            rpc_id: 9,
            method: "event.poll".to_string(),
            params: json!({ "id": "owner" }),
        };

        assert!(workflow_poll_event_tool_call(&request).is_err());
    }
}
