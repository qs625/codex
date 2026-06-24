use std::collections::BTreeMap;
use std::collections::HashMap;

use codex_config_types::AppToolApproval;
use codex_mcp_tool_types::ToolAnnotations;
use codex_protocol::mcp_approval_meta::APPROVAL_KIND_KEY as MCP_TOOL_APPROVAL_KIND_KEY;
use codex_protocol::mcp_approval_meta::APPROVAL_KIND_MCP_TOOL_CALL as MCP_TOOL_APPROVAL_KIND_MCP_TOOL_CALL;
use codex_protocol::mcp_approval_meta::CONNECTOR_DESCRIPTION_KEY as MCP_TOOL_APPROVAL_CONNECTOR_DESCRIPTION_KEY;
use codex_protocol::mcp_approval_meta::CONNECTOR_ID_KEY as MCP_TOOL_APPROVAL_CONNECTOR_ID_KEY;
use codex_protocol::mcp_approval_meta::CONNECTOR_NAME_KEY as MCP_TOOL_APPROVAL_CONNECTOR_NAME_KEY;
use codex_protocol::mcp_approval_meta::PERSIST_ALWAYS as MCP_TOOL_APPROVAL_PERSIST_ALWAYS;
use codex_protocol::mcp_approval_meta::PERSIST_KEY as MCP_TOOL_APPROVAL_PERSIST_KEY;
use codex_protocol::mcp_approval_meta::PERSIST_SESSION as MCP_TOOL_APPROVAL_PERSIST_SESSION;
use codex_protocol::mcp_approval_meta::SOURCE_CONNECTOR as MCP_TOOL_APPROVAL_SOURCE_CONNECTOR;
use codex_protocol::mcp_approval_meta::SOURCE_KEY as MCP_TOOL_APPROVAL_SOURCE_KEY;
use codex_protocol::mcp_approval_meta::TOOL_DESCRIPTION_KEY as MCP_TOOL_APPROVAL_TOOL_DESCRIPTION_KEY;
use codex_protocol::mcp_approval_meta::TOOL_PARAMS_DISPLAY_KEY as MCP_TOOL_APPROVAL_TOOL_PARAMS_DISPLAY_KEY;
use codex_protocol::mcp_approval_meta::TOOL_PARAMS_KEY as MCP_TOOL_APPROVAL_TOOL_PARAMS_KEY;
use codex_protocol::mcp_approval_meta::TOOL_TITLE_KEY as MCP_TOOL_APPROVAL_TOOL_TITLE_KEY;
use codex_protocol::protocol::McpInvocation;
use codex_protocol::request_user_input::RequestUserInputAnswer;
use codex_protocol::request_user_input::RequestUserInputQuestion;
use codex_protocol::request_user_input::RequestUserInputQuestionOption;
use codex_protocol::request_user_input::RequestUserInputResponse;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;

use crate::CODEX_APPS_MCP_SERVER_NAME;
use crate::ElicitationAction;
use crate::ElicitationResponse;
use crate::McpElicitationObjectType;
use crate::McpElicitationSchema;
use crate::McpServerElicitationRequest;
use crate::McpServerElicitationRequestParams;
use crate::tool_approval_templates::RenderedMcpToolApprovalParam;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpToolApprovalDecision {
    Accept,
    AcceptForSession,
    AcceptAndRemember,
    Decline { message: Option<String> },
    Cancel,
    BlockedBySafetyMonitor(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct McpToolApprovalMetadata {
    pub annotations: Option<ToolAnnotations>,
    pub connector_id: Option<String>,
    pub connector_name: Option<String>,
    pub connector_description: Option<String>,
    pub tool_title: Option<String>,
    pub tool_description: Option<String>,
    pub mcp_app_resource_uri: Option<String>,
    pub codex_apps_meta: Option<Map<String, Value>>,
    pub openai_file_input_params: Option<Vec<String>>,
}

#[derive(Clone, Copy)]
pub struct McpToolApprovalPromptOptions {
    pub allow_session_remember: bool,
    pub allow_persistent_approval: bool,
}

pub struct McpToolApprovalElicitationRequest<'a> {
    pub thread_id: &'a str,
    pub turn_id: Option<&'a str>,
    pub server: &'a str,
    pub metadata: Option<&'a McpToolApprovalMetadata>,
    pub tool_params: Option<&'a Value>,
    pub tool_params_display: Option<&'a [RenderedMcpToolApprovalParam]>,
    pub question: RequestUserInputQuestion,
    pub message_override: Option<&'a str>,
    pub prompt_options: McpToolApprovalPromptOptions,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct McpToolApprovalKey {
    pub server: String,
    pub connector_id: Option<String>,
    pub tool_name: String,
}

pub const MCP_TOOL_APPROVAL_QUESTION_ID_PREFIX: &str = "mcp_tool_call_approval";
pub const MCP_TOOL_APPROVAL_ACCEPT: &str = "Allow";
pub const MCP_TOOL_APPROVAL_ACCEPT_FOR_SESSION: &str = "Allow for this session";
pub const MCP_TOOL_APPROVAL_DECLINE_SYNTHETIC: &str = "__codex_mcp_decline__";
pub const MCP_TOOL_APPROVAL_ACCEPT_AND_REMEMBER: &str = "Allow and don't ask me again";
pub const MCP_TOOL_APPROVAL_CANCEL: &str = "Cancel";

pub fn is_mcp_tool_approval_question_id(question_id: &str) -> bool {
    question_id
        .strip_prefix(MCP_TOOL_APPROVAL_QUESTION_ID_PREFIX)
        .is_some_and(|suffix| suffix.starts_with('_'))
}

pub fn mcp_tool_approval_prompt_options(
    session_approval_key: Option<&McpToolApprovalKey>,
    persistent_approval_key: Option<&McpToolApprovalKey>,
    tool_call_mcp_elicitation_enabled: bool,
) -> McpToolApprovalPromptOptions {
    McpToolApprovalPromptOptions {
        allow_session_remember: session_approval_key.is_some(),
        allow_persistent_approval: tool_call_mcp_elicitation_enabled
            && persistent_approval_key.is_some(),
    }
}

pub fn session_mcp_tool_approval_key(
    invocation: &McpInvocation,
    metadata: Option<&McpToolApprovalMetadata>,
    approval_mode: AppToolApproval,
) -> Option<McpToolApprovalKey> {
    if approval_mode != AppToolApproval::Auto {
        return None;
    }

    let connector_id = metadata.and_then(|metadata| metadata.connector_id.clone());
    if invocation.server == CODEX_APPS_MCP_SERVER_NAME && connector_id.is_none() {
        return None;
    }

    Some(McpToolApprovalKey {
        server: invocation.server.clone(),
        connector_id,
        tool_name: invocation.tool.clone(),
    })
}

pub fn persistent_mcp_tool_approval_key(
    invocation: &McpInvocation,
    metadata: Option<&McpToolApprovalMetadata>,
    approval_mode: AppToolApproval,
) -> Option<McpToolApprovalKey> {
    session_mcp_tool_approval_key(invocation, metadata, approval_mode)
}

pub fn requires_mcp_tool_approval(annotations: Option<&ToolAnnotations>) -> bool {
    let destructive_hint = annotations.and_then(|annotations| annotations.destructive_hint);
    if destructive_hint == Some(true) {
        return true;
    }

    let read_only_hint = annotations
        .and_then(|annotations| annotations.read_only_hint)
        .unwrap_or(false);
    if read_only_hint {
        return false;
    }

    destructive_hint.unwrap_or(true)
        || annotations
            .and_then(|annotations| annotations.open_world_hint)
            .unwrap_or(true)
}

pub fn build_mcp_tool_approval_question(
    question_id: String,
    server: &str,
    tool_name: &str,
    connector_name: Option<&str>,
    prompt_options: McpToolApprovalPromptOptions,
    question_override: Option<&str>,
) -> RequestUserInputQuestion {
    let question = question_override
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            build_mcp_tool_approval_fallback_message(server, tool_name, connector_name)
        });
    let question = format!("{}?", question.trim_end_matches('?'));

    let mut options = vec![RequestUserInputQuestionOption {
        label: MCP_TOOL_APPROVAL_ACCEPT.to_string(),
        description: "Run the tool and continue.".to_string(),
    }];
    if prompt_options.allow_session_remember {
        options.push(RequestUserInputQuestionOption {
            label: MCP_TOOL_APPROVAL_ACCEPT_FOR_SESSION.to_string(),
            description: "Run the tool and remember this choice for this session.".to_string(),
        });
    }
    if prompt_options.allow_persistent_approval {
        options.push(RequestUserInputQuestionOption {
            label: MCP_TOOL_APPROVAL_ACCEPT_AND_REMEMBER.to_string(),
            description: "Run the tool and remember this choice for future tool calls.".to_string(),
        });
    }
    options.push(RequestUserInputQuestionOption {
        label: MCP_TOOL_APPROVAL_CANCEL.to_string(),
        description: "Cancel this tool call.".to_string(),
    });

    RequestUserInputQuestion {
        id: question_id,
        header: "Approve app tool call?".to_string(),
        question,
        is_other: false,
        is_secret: false,
        options: Some(options),
    }
}

fn build_mcp_tool_approval_fallback_message(
    server: &str,
    tool_name: &str,
    connector_name: Option<&str>,
) -> String {
    let actor = connector_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            if server == CODEX_APPS_MCP_SERVER_NAME {
                "this app".to_string()
            } else {
                format!("the {server} MCP server")
            }
        });
    format!("Allow {actor} to run tool \"{tool_name}\"?")
}

pub fn mcp_tool_approval_question_text(question: String, monitor_reason: Option<&str>) -> String {
    match monitor_reason.map(str::trim) {
        Some(reason) if !reason.is_empty() => {
            format!("Tool call needs your approval. Reason: {reason}")
        }
        _ => question,
    }
}

pub fn build_mcp_tool_approval_elicitation_request(
    request: McpToolApprovalElicitationRequest<'_>,
) -> McpServerElicitationRequestParams {
    let message = request
        .message_override
        .map(ToString::to_string)
        .unwrap_or_else(|| request.question.question.clone());

    McpServerElicitationRequestParams {
        thread_id: request.thread_id.to_string(),
        turn_id: request.turn_id.map(str::to_string),
        server_name: request.server.to_string(),
        request: McpServerElicitationRequest::Form {
            meta: build_mcp_tool_approval_elicitation_meta(
                request.server,
                request.metadata,
                request.tool_params,
                request.tool_params_display,
                request.prompt_options,
            ),
            message,
            requested_schema: McpElicitationSchema {
                schema_uri: None,
                type_: McpElicitationObjectType::Object,
                properties: BTreeMap::new(),
                required: None,
            },
        },
    }
}

pub fn build_mcp_tool_approval_elicitation_meta(
    server: &str,
    metadata: Option<&McpToolApprovalMetadata>,
    tool_params: Option<&Value>,
    tool_params_display: Option<&[RenderedMcpToolApprovalParam]>,
    prompt_options: McpToolApprovalPromptOptions,
) -> Option<Value> {
    let mut meta = Map::new();
    meta.insert(
        MCP_TOOL_APPROVAL_KIND_KEY.to_string(),
        Value::String(MCP_TOOL_APPROVAL_KIND_MCP_TOOL_CALL.to_string()),
    );
    match (
        prompt_options.allow_session_remember,
        prompt_options.allow_persistent_approval,
    ) {
        (true, true) => {
            meta.insert(
                MCP_TOOL_APPROVAL_PERSIST_KEY.to_string(),
                serde_json::json!([
                    MCP_TOOL_APPROVAL_PERSIST_SESSION,
                    MCP_TOOL_APPROVAL_PERSIST_ALWAYS,
                ]),
            );
        }
        (true, false) => {
            meta.insert(
                MCP_TOOL_APPROVAL_PERSIST_KEY.to_string(),
                Value::String(MCP_TOOL_APPROVAL_PERSIST_SESSION.to_string()),
            );
        }
        (false, true) => {
            meta.insert(
                MCP_TOOL_APPROVAL_PERSIST_KEY.to_string(),
                Value::String(MCP_TOOL_APPROVAL_PERSIST_ALWAYS.to_string()),
            );
        }
        (false, false) => {}
    }
    if let Some(metadata) = metadata {
        if let Some(tool_title) = metadata.tool_title.as_ref() {
            meta.insert(
                MCP_TOOL_APPROVAL_TOOL_TITLE_KEY.to_string(),
                Value::String(tool_title.clone()),
            );
        }
        if let Some(tool_description) = metadata.tool_description.as_ref() {
            meta.insert(
                MCP_TOOL_APPROVAL_TOOL_DESCRIPTION_KEY.to_string(),
                Value::String(tool_description.clone()),
            );
        }
        if server == CODEX_APPS_MCP_SERVER_NAME
            && (metadata.connector_id.is_some()
                || metadata.connector_name.is_some()
                || metadata.connector_description.is_some())
        {
            meta.insert(
                MCP_TOOL_APPROVAL_SOURCE_KEY.to_string(),
                Value::String(MCP_TOOL_APPROVAL_SOURCE_CONNECTOR.to_string()),
            );
            if let Some(connector_id) = metadata.connector_id.as_deref() {
                meta.insert(
                    MCP_TOOL_APPROVAL_CONNECTOR_ID_KEY.to_string(),
                    Value::String(connector_id.to_string()),
                );
            }
            if let Some(connector_name) = metadata.connector_name.as_ref() {
                meta.insert(
                    MCP_TOOL_APPROVAL_CONNECTOR_NAME_KEY.to_string(),
                    Value::String(connector_name.clone()),
                );
            }
            if let Some(connector_description) = metadata.connector_description.as_ref() {
                meta.insert(
                    MCP_TOOL_APPROVAL_CONNECTOR_DESCRIPTION_KEY.to_string(),
                    Value::String(connector_description.clone()),
                );
            }
        }
    }
    if let Some(tool_params) = tool_params {
        meta.insert(
            MCP_TOOL_APPROVAL_TOOL_PARAMS_KEY.to_string(),
            tool_params.clone(),
        );
    }
    if let Some(tool_params_display) = tool_params_display
        && let Ok(tool_params_display) = serde_json::to_value(tool_params_display)
    {
        meta.insert(
            MCP_TOOL_APPROVAL_TOOL_PARAMS_DISPLAY_KEY.to_string(),
            tool_params_display,
        );
    }
    (!meta.is_empty()).then_some(Value::Object(meta))
}

pub fn build_mcp_tool_approval_display_params(
    tool_params: Option<&Value>,
) -> Option<Vec<RenderedMcpToolApprovalParam>> {
    let tool_params = tool_params?.as_object()?;
    let mut display_params = tool_params
        .iter()
        .map(|(name, value)| RenderedMcpToolApprovalParam {
            name: name.clone(),
            value: value.clone(),
            display_name: name.clone(),
        })
        .collect::<Vec<_>>();
    display_params.sort_by(|left, right| left.name.cmp(&right.name));
    Some(display_params)
}

pub fn parse_mcp_tool_approval_elicitation_response(
    response: Option<ElicitationResponse>,
    question_id: &str,
) -> McpToolApprovalDecision {
    let Some(response) = response else {
        return McpToolApprovalDecision::Cancel;
    };
    match response.action {
        ElicitationAction::Accept => {
            match response
                .meta
                .as_ref()
                .and_then(Value::as_object)
                .and_then(|meta| meta.get(MCP_TOOL_APPROVAL_PERSIST_KEY))
                .and_then(Value::as_str)
            {
                Some(MCP_TOOL_APPROVAL_PERSIST_SESSION) => {
                    return McpToolApprovalDecision::AcceptForSession;
                }
                Some(MCP_TOOL_APPROVAL_PERSIST_ALWAYS) => {
                    return McpToolApprovalDecision::AcceptAndRemember;
                }
                _ => {}
            }

            match parse_mcp_tool_approval_response(
                request_user_input_response_from_elicitation_content(response.content),
                question_id,
            ) {
                McpToolApprovalDecision::Cancel => McpToolApprovalDecision::Accept,
                decision => decision,
            }
        }
        ElicitationAction::Decline => McpToolApprovalDecision::Decline { message: None },
        ElicitationAction::Cancel => McpToolApprovalDecision::Cancel,
    }
}

fn request_user_input_response_from_elicitation_content(
    content: Option<Value>,
) -> Option<RequestUserInputResponse> {
    let Some(content) = content else {
        return Some(RequestUserInputResponse {
            answers: HashMap::new(),
        });
    };
    let content = content.as_object()?;
    let answers = content
        .iter()
        .filter_map(|(question_id, value)| {
            let answers = match value {
                Value::String(answer) => vec![answer.clone()],
                Value::Array(values) => values
                    .iter()
                    .filter_map(|value| value.as_str().map(ToString::to_string))
                    .collect(),
                _ => return None,
            };
            Some((question_id.clone(), RequestUserInputAnswer { answers }))
        })
        .collect();

    Some(RequestUserInputResponse { answers })
}

pub fn parse_mcp_tool_approval_response(
    response: Option<RequestUserInputResponse>,
    question_id: &str,
) -> McpToolApprovalDecision {
    let Some(response) = response else {
        return McpToolApprovalDecision::Cancel;
    };
    let answers = response
        .answers
        .get(question_id)
        .map(|answer| answer.answers.as_slice());
    let Some(answers) = answers else {
        return McpToolApprovalDecision::Cancel;
    };
    if answers
        .iter()
        .any(|answer| answer == MCP_TOOL_APPROVAL_DECLINE_SYNTHETIC)
    {
        McpToolApprovalDecision::Decline { message: None }
    } else if answers
        .iter()
        .any(|answer| answer == MCP_TOOL_APPROVAL_ACCEPT_FOR_SESSION)
    {
        McpToolApprovalDecision::AcceptForSession
    } else if answers
        .iter()
        .any(|answer| answer == MCP_TOOL_APPROVAL_ACCEPT_AND_REMEMBER)
    {
        McpToolApprovalDecision::AcceptAndRemember
    } else if answers
        .iter()
        .any(|answer| answer == MCP_TOOL_APPROVAL_ACCEPT)
    {
        McpToolApprovalDecision::Accept
    } else {
        McpToolApprovalDecision::Cancel
    }
}

pub fn normalize_approval_decision_for_mode(
    decision: McpToolApprovalDecision,
    approval_mode: AppToolApproval,
) -> McpToolApprovalDecision {
    if approval_mode == AppToolApproval::Prompt
        && matches!(
            decision,
            McpToolApprovalDecision::AcceptForSession | McpToolApprovalDecision::AcceptAndRemember
        )
    {
        McpToolApprovalDecision::Accept
    } else {
        decision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn prompt_options(
        allow_session_remember: bool,
        allow_persistent_approval: bool,
    ) -> McpToolApprovalPromptOptions {
        McpToolApprovalPromptOptions {
            allow_session_remember,
            allow_persistent_approval,
        }
    }

    fn approval_metadata(
        connector_id: Option<&str>,
        connector_name: Option<&str>,
        connector_description: Option<&str>,
        tool_title: Option<&str>,
        tool_description: Option<&str>,
    ) -> McpToolApprovalMetadata {
        McpToolApprovalMetadata {
            annotations: None,
            connector_id: connector_id.map(str::to_string),
            connector_name: connector_name.map(str::to_string),
            connector_description: connector_description.map(str::to_string),
            tool_title: tool_title.map(str::to_string),
            tool_description: tool_description.map(str::to_string),
            mcp_app_resource_uri: None,
            codex_apps_meta: None,
            openai_file_input_params: None,
        }
    }

    #[test]
    fn prompt_mode_does_not_allow_persistent_remember() {
        assert_eq!(
            normalize_approval_decision_for_mode(
                McpToolApprovalDecision::AcceptForSession,
                AppToolApproval::Prompt,
            ),
            McpToolApprovalDecision::Accept
        );
        assert_eq!(
            normalize_approval_decision_for_mode(
                McpToolApprovalDecision::AcceptAndRemember,
                AppToolApproval::Prompt,
            ),
            McpToolApprovalDecision::Accept
        );
    }

    #[test]
    fn approval_question_text_prepends_safety_reason() {
        assert_eq!(
            mcp_tool_approval_question_text(
                "Allow this action?".to_string(),
                Some("This tool may contact an external system."),
            ),
            "Tool call needs your approval. Reason: This tool may contact an external system."
        );
    }

    #[test]
    fn approval_elicitation_request_uses_message_override_and_preserves_tool_params_keys() {
        let question = build_mcp_tool_approval_question(
            "q".to_string(),
            CODEX_APPS_MCP_SERVER_NAME,
            "create_event",
            Some("Calendar"),
            prompt_options(
                /*allow_session_remember*/ true, /*allow_persistent_approval*/ true,
            ),
            Some("Allow Calendar to create an event?"),
        );

        let request =
            build_mcp_tool_approval_elicitation_request(McpToolApprovalElicitationRequest {
                thread_id: "thread-123",
                turn_id: Some("turn-123"),
                server: CODEX_APPS_MCP_SERVER_NAME,
                metadata: Some(&approval_metadata(
                    Some("calendar"),
                    Some("Calendar"),
                    Some("Manage events and schedules."),
                    Some("Create Event"),
                    Some("Create a calendar event."),
                )),
                tool_params: Some(&serde_json::json!({
                    "calendar_id": "primary",
                    "title": "Roadmap review",
                })),
                tool_params_display: Some(&[
                    RenderedMcpToolApprovalParam {
                        name: "calendar_id".to_string(),
                        value: serde_json::json!("primary"),
                        display_name: "Calendar".to_string(),
                    },
                    RenderedMcpToolApprovalParam {
                        name: "title".to_string(),
                        value: serde_json::json!("Roadmap review"),
                        display_name: "Title".to_string(),
                    },
                ]),
                question,
                message_override: Some("Allow Calendar to create an event?"),
                prompt_options: prompt_options(
                    /*allow_session_remember*/ true, /*allow_persistent_approval*/ true,
                ),
            });

        assert_eq!(
            request,
            McpServerElicitationRequestParams {
                thread_id: "thread-123".to_string(),
                turn_id: Some("turn-123".to_string()),
                server_name: CODEX_APPS_MCP_SERVER_NAME.to_string(),
                request: McpServerElicitationRequest::Form {
                    meta: Some(serde_json::json!({
                        MCP_TOOL_APPROVAL_KIND_KEY: MCP_TOOL_APPROVAL_KIND_MCP_TOOL_CALL,
                        MCP_TOOL_APPROVAL_PERSIST_KEY: [
                            MCP_TOOL_APPROVAL_PERSIST_SESSION,
                            MCP_TOOL_APPROVAL_PERSIST_ALWAYS,
                        ],
                        MCP_TOOL_APPROVAL_SOURCE_KEY: MCP_TOOL_APPROVAL_SOURCE_CONNECTOR,
                        MCP_TOOL_APPROVAL_CONNECTOR_ID_KEY: "calendar",
                        MCP_TOOL_APPROVAL_CONNECTOR_NAME_KEY: "Calendar",
                        MCP_TOOL_APPROVAL_CONNECTOR_DESCRIPTION_KEY: "Manage events and schedules.",
                        MCP_TOOL_APPROVAL_TOOL_TITLE_KEY: "Create Event",
                        MCP_TOOL_APPROVAL_TOOL_DESCRIPTION_KEY: "Create a calendar event.",
                        MCP_TOOL_APPROVAL_TOOL_PARAMS_KEY: {
                            "calendar_id": "primary",
                            "title": "Roadmap review",
                        },
                        MCP_TOOL_APPROVAL_TOOL_PARAMS_DISPLAY_KEY: [
                            {
                                "name": "calendar_id",
                                "value": "primary",
                                "display_name": "Calendar",
                            },
                            {
                                "name": "title",
                                "value": "Roadmap review",
                                "display_name": "Title",
                            },
                        ],
                    })),
                    message: "Allow Calendar to create an event?".to_string(),
                    requested_schema: McpElicitationSchema {
                        schema_uri: None,
                        type_: McpElicitationObjectType::Object,
                        properties: BTreeMap::new(),
                        required: None,
                    },
                },
            }
        );
    }

    #[test]
    fn mcp_tool_question_labels_and_options_follow_server_and_prompt_policy() {
        let custom_question = build_mcp_tool_approval_question(
            "q".to_string(),
            "custom_server",
            "run_action",
            /*connector_name*/ None,
            prompt_options(
                /*allow_session_remember*/ false, /*allow_persistent_approval*/ false,
            ),
            /*question_override*/ None,
        );
        assert_eq!(custom_question.header, "Approve app tool call?");
        assert_eq!(
            custom_question.question,
            "Allow the custom_server MCP server to run tool \"run_action\"?"
        );
        assert!(
            !custom_question
                .options
                .expect("options")
                .into_iter()
                .map(|option| option.label)
                .any(|label| label == MCP_TOOL_APPROVAL_ACCEPT_AND_REMEMBER)
        );

        let app_question = build_mcp_tool_approval_question(
            "q".to_string(),
            CODEX_APPS_MCP_SERVER_NAME,
            "run_action",
            /*connector_name*/ None,
            prompt_options(
                /*allow_session_remember*/ true, /*allow_persistent_approval*/ true,
            ),
            /*question_override*/ None,
        );
        assert_eq!(
            app_question.question,
            "Allow this app to run tool \"run_action\"?"
        );

        let trusted_question = build_mcp_tool_approval_question(
            "q".to_string(),
            CODEX_APPS_MCP_SERVER_NAME,
            "run_action",
            Some("Calendar"),
            prompt_options(
                /*allow_session_remember*/ true, /*allow_persistent_approval*/ true,
            ),
            /*question_override*/ None,
        );
        assert_eq!(
            trusted_question
                .options
                .expect("options")
                .into_iter()
                .map(|option| option.label)
                .collect::<Vec<_>>(),
            vec![
                MCP_TOOL_APPROVAL_ACCEPT.to_string(),
                MCP_TOOL_APPROVAL_ACCEPT_FOR_SESSION.to_string(),
                MCP_TOOL_APPROVAL_ACCEPT_AND_REMEMBER.to_string(),
                MCP_TOOL_APPROVAL_CANCEL.to_string(),
            ]
        );
    }

    #[test]
    fn mcp_tool_prompt_options_can_disable_persistent_approval() {
        let session_key = McpToolApprovalKey {
            server: CODEX_APPS_MCP_SERVER_NAME.to_string(),
            connector_id: Some("calendar".to_string()),
            tool_name: "run_action".to_string(),
        };
        let persistent_key = session_key.clone();
        let question = build_mcp_tool_approval_question(
            "q".to_string(),
            CODEX_APPS_MCP_SERVER_NAME,
            "run_action",
            Some("Calendar"),
            mcp_tool_approval_prompt_options(
                Some(&session_key),
                Some(&persistent_key),
                /*tool_call_mcp_elicitation_enabled*/ false,
            ),
            /*question_override*/ None,
        );

        assert_eq!(
            question
                .options
                .expect("options")
                .into_iter()
                .map(|option| option.label)
                .collect::<Vec<_>>(),
            vec![
                MCP_TOOL_APPROVAL_ACCEPT.to_string(),
                MCP_TOOL_APPROVAL_ACCEPT_FOR_SESSION.to_string(),
                MCP_TOOL_APPROVAL_CANCEL.to_string(),
            ]
        );
    }

    #[test]
    fn mcp_tool_approval_keys_support_custom_servers_and_codex_apps_connectors() {
        let custom_invocation = McpInvocation {
            server: "custom_server".to_string(),
            tool: "run_action".to_string(),
            arguments: None,
        };
        let custom_expected = McpToolApprovalKey {
            server: "custom_server".to_string(),
            connector_id: None,
            tool_name: "run_action".to_string(),
        };
        assert_eq!(
            session_mcp_tool_approval_key(
                &custom_invocation,
                /*metadata*/ None,
                AppToolApproval::Auto,
            ),
            Some(custom_expected.clone())
        );
        assert_eq!(
            persistent_mcp_tool_approval_key(
                &custom_invocation,
                /*metadata*/ None,
                AppToolApproval::Auto,
            ),
            Some(custom_expected)
        );

        let app_invocation = McpInvocation {
            server: CODEX_APPS_MCP_SERVER_NAME.to_string(),
            tool: "calendar/list_events".to_string(),
            arguments: None,
        };
        let app_metadata = approval_metadata(
            Some("calendar"),
            Some("Calendar"),
            /*connector_description*/ None,
            /*tool_title*/ None,
            /*tool_description*/ None,
        );
        let app_expected = McpToolApprovalKey {
            server: CODEX_APPS_MCP_SERVER_NAME.to_string(),
            connector_id: Some("calendar".to_string()),
            tool_name: "calendar/list_events".to_string(),
        };
        assert_eq!(
            session_mcp_tool_approval_key(
                &app_invocation,
                Some(&app_metadata),
                AppToolApproval::Auto,
            ),
            Some(app_expected.clone())
        );
        assert_eq!(
            persistent_mcp_tool_approval_key(
                &app_invocation,
                Some(&app_metadata),
                AppToolApproval::Auto,
            ),
            Some(app_expected)
        );
    }

    #[test]
    fn mcp_tool_approval_elicitation_meta_carries_persist_and_connector_details() {
        assert_eq!(
            build_mcp_tool_approval_elicitation_meta(
                "custom_server",
                /*metadata*/ None,
                /*tool_params*/ None,
                /*tool_params_display*/ None,
                prompt_options(
                    /*allow_session_remember*/ false,
                    /*allow_persistent_approval*/ false,
                ),
            ),
            Some(serde_json::json!({
                MCP_TOOL_APPROVAL_KIND_KEY: MCP_TOOL_APPROVAL_KIND_MCP_TOOL_CALL,
            }))
        );

        assert_eq!(
            build_mcp_tool_approval_elicitation_meta(
                "custom_server",
                Some(&approval_metadata(
                    /*connector_id*/ None,
                    /*connector_name*/ None,
                    /*connector_description*/ None,
                    Some("Run Action"),
                    Some("Runs the selected action."),
                )),
                Some(&serde_json::json!({"id": 1})),
                /*tool_params_display*/ None,
                prompt_options(
                    /*allow_session_remember*/ true, /*allow_persistent_approval*/ true,
                ),
            ),
            Some(serde_json::json!({
                MCP_TOOL_APPROVAL_KIND_KEY: MCP_TOOL_APPROVAL_KIND_MCP_TOOL_CALL,
                MCP_TOOL_APPROVAL_PERSIST_KEY: [
                    MCP_TOOL_APPROVAL_PERSIST_SESSION,
                    MCP_TOOL_APPROVAL_PERSIST_ALWAYS,
                ],
                MCP_TOOL_APPROVAL_TOOL_TITLE_KEY: "Run Action",
                MCP_TOOL_APPROVAL_TOOL_DESCRIPTION_KEY: "Runs the selected action.",
                MCP_TOOL_APPROVAL_TOOL_PARAMS_KEY: {
                    "id": 1,
                },
            }))
        );

        assert_eq!(
            build_mcp_tool_approval_elicitation_meta(
                CODEX_APPS_MCP_SERVER_NAME,
                Some(&approval_metadata(
                    Some("calendar"),
                    Some("Calendar"),
                    Some("Manage events and schedules."),
                    Some("Run Action"),
                    Some("Runs the selected action."),
                )),
                Some(&serde_json::json!({
                    "calendar_id": "primary",
                })),
                /*tool_params_display*/ None,
                prompt_options(
                    /*allow_session_remember*/ true, /*allow_persistent_approval*/ true,
                ),
            ),
            Some(serde_json::json!({
                MCP_TOOL_APPROVAL_KIND_KEY: MCP_TOOL_APPROVAL_KIND_MCP_TOOL_CALL,
                MCP_TOOL_APPROVAL_PERSIST_KEY: [
                    MCP_TOOL_APPROVAL_PERSIST_SESSION,
                    MCP_TOOL_APPROVAL_PERSIST_ALWAYS,
                ],
                MCP_TOOL_APPROVAL_SOURCE_KEY: MCP_TOOL_APPROVAL_SOURCE_CONNECTOR,
                MCP_TOOL_APPROVAL_CONNECTOR_ID_KEY: "calendar",
                MCP_TOOL_APPROVAL_CONNECTOR_NAME_KEY: "Calendar",
                MCP_TOOL_APPROVAL_CONNECTOR_DESCRIPTION_KEY: "Manage events and schedules.",
                MCP_TOOL_APPROVAL_TOOL_TITLE_KEY: "Run Action",
                MCP_TOOL_APPROVAL_TOOL_DESCRIPTION_KEY: "Runs the selected action.",
                MCP_TOOL_APPROVAL_TOOL_PARAMS_KEY: {
                    "calendar_id": "primary",
                },
            }))
        );
    }

    #[test]
    fn parse_mcp_tool_approval_responses_preserve_decline_and_persist_choices() {
        assert_eq!(
            parse_mcp_tool_approval_elicitation_response(
                Some(ElicitationResponse {
                    action: ElicitationAction::Decline,
                    content: Some(serde_json::json!({
                        "approval": MCP_TOOL_APPROVAL_ACCEPT,
                    })),
                    meta: None,
                }),
                "approval",
            ),
            McpToolApprovalDecision::Decline { message: None }
        );
        assert_eq!(
            parse_mcp_tool_approval_response(
                Some(RequestUserInputResponse {
                    answers: HashMap::from([(
                        "approval".to_string(),
                        RequestUserInputAnswer {
                            answers: vec![MCP_TOOL_APPROVAL_DECLINE_SYNTHETIC.to_string()],
                        },
                    )]),
                }),
                "approval",
            ),
            McpToolApprovalDecision::Decline { message: None }
        );
        assert_eq!(
            parse_mcp_tool_approval_elicitation_response(
                Some(ElicitationResponse {
                    action: ElicitationAction::Accept,
                    content: None,
                    meta: Some(serde_json::json!({
                        MCP_TOOL_APPROVAL_PERSIST_KEY: MCP_TOOL_APPROVAL_PERSIST_ALWAYS,
                    })),
                }),
                "approval",
            ),
            McpToolApprovalDecision::AcceptAndRemember
        );
        assert_eq!(
            parse_mcp_tool_approval_elicitation_response(
                Some(ElicitationResponse {
                    action: ElicitationAction::Accept,
                    content: None,
                    meta: Some(serde_json::json!({
                        MCP_TOOL_APPROVAL_PERSIST_KEY: MCP_TOOL_APPROVAL_PERSIST_SESSION,
                    })),
                }),
                "approval",
            ),
            McpToolApprovalDecision::AcceptForSession
        );
        assert_eq!(
            parse_mcp_tool_approval_elicitation_response(
                Some(ElicitationResponse {
                    action: ElicitationAction::Accept,
                    content: None,
                    meta: None,
                }),
                "approval",
            ),
            McpToolApprovalDecision::Accept
        );
        assert_eq!(
            parse_mcp_tool_approval_elicitation_response(
                Some(ElicitationResponse {
                    action: ElicitationAction::Accept,
                    content: Some(serde_json::json!({
                        "approval": MCP_TOOL_APPROVAL_ACCEPT_AND_REMEMBER,
                    })),
                    meta: None,
                }),
                "approval",
            ),
            McpToolApprovalDecision::AcceptAndRemember
        );
    }
}
