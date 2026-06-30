use codex_guardian::GuardianApprovalRequest;
use codex_mcp_types::ElicitationAction;
use codex_mcp_types::ElicitationResponse;
use codex_mcp_types::ElicitationReviewRequest;
use codex_protocol::config_types::ApprovalsReviewer;
use codex_protocol::mcp_approval_meta::APPROVAL_KIND_KEY as MCP_ELICITATION_APPROVAL_KIND_KEY;
use codex_protocol::mcp_approval_meta::APPROVAL_KIND_MCP_TOOL_CALL as MCP_ELICITATION_APPROVAL_KIND_MCP_TOOL_CALL;
use codex_protocol::mcp_approval_meta::APPROVALS_REVIEWER_KEY as MCP_ELICITATION_APPROVALS_REVIEWER_KEY;
use codex_protocol::mcp_approval_meta::CONNECTOR_DESCRIPTION_KEY as MCP_ELICITATION_CONNECTOR_DESCRIPTION_KEY;
use codex_protocol::mcp_approval_meta::CONNECTOR_ID_KEY as MCP_ELICITATION_CONNECTOR_ID_KEY;
use codex_protocol::mcp_approval_meta::CONNECTOR_NAME_KEY as MCP_ELICITATION_CONNECTOR_NAME_KEY;
use codex_protocol::mcp_approval_meta::REQUEST_TYPE_APPROVAL_REQUEST as MCP_ELICITATION_REQUEST_TYPE_APPROVAL_REQUEST;
use codex_protocol::mcp_approval_meta::REQUEST_TYPE_KEY as MCP_ELICITATION_REQUEST_TYPE_KEY;
use codex_protocol::mcp_approval_meta::TOOL_DESCRIPTION_KEY as MCP_ELICITATION_TOOL_DESCRIPTION_KEY;
use codex_protocol::mcp_approval_meta::TOOL_NAME_KEY as MCP_ELICITATION_TOOL_NAME_KEY;
use codex_protocol::mcp_approval_meta::TOOL_PARAMS_KEY as MCP_ELICITATION_TOOL_PARAMS_KEY;
use codex_protocol::mcp_approval_meta::TOOL_TITLE_KEY as MCP_ELICITATION_TOOL_TITLE_KEY;
use codex_protocol::protocol::ReviewDecision;
use serde_json::Map;
use serde_json::Value;

const MCP_ELICITATION_DECLINE_MESSAGE_KEY: &str = "message";

#[derive(Debug, PartialEq)]
pub enum GuardianElicitationReview {
    NotRequested,
    Decline(&'static str),
    ApprovalRequest(Box<GuardianApprovalRequest>),
}

pub fn guardian_elicitation_review_request(
    request: &ElicitationReviewRequest,
) -> GuardianElicitationReview {
    let (meta, requested_schema) = match &request.elicitation {
        codex_protocol::approvals::ElicitationRequest::Form {
            meta,
            requested_schema,
            ..
        } => (meta, Some(requested_schema)),
        codex_protocol::approvals::ElicitationRequest::Url { meta, .. } => {
            return if meta_requests_approval_request(meta) {
                GuardianElicitationReview::Decline(
                    "guardian MCP elicitation review only supports form elicitations",
                )
            } else {
                GuardianElicitationReview::NotRequested
            };
        }
    };

    let Some(meta) = meta.as_ref().and_then(Value::as_object) else {
        return GuardianElicitationReview::NotRequested;
    };
    if metadata_str(meta, MCP_ELICITATION_REQUEST_TYPE_KEY)
        != Some(MCP_ELICITATION_REQUEST_TYPE_APPROVAL_REQUEST)
    {
        return GuardianElicitationReview::NotRequested;
    }
    if metadata_str(meta, MCP_ELICITATION_APPROVAL_KIND_KEY)
        != Some(MCP_ELICITATION_APPROVAL_KIND_MCP_TOOL_CALL)
    {
        return GuardianElicitationReview::Decline(
            "guardian MCP elicitation metadata must declare mcp_tool_call approval kind",
        );
    }
    if requested_schema.is_some_and(requested_schema_has_properties) {
        return GuardianElicitationReview::Decline(
            "guardian MCP elicitation review only supports empty form schemas",
        );
    }

    let Some(tool_name) = metadata_owned_string(meta, MCP_ELICITATION_TOOL_NAME_KEY) else {
        return GuardianElicitationReview::Decline(
            "guardian MCP elicitation metadata must include a non-empty tool_name",
        );
    };
    let arguments = match meta.get(MCP_ELICITATION_TOOL_PARAMS_KEY) {
        Some(value @ Value::Object(_)) => Some(value.clone()),
        Some(_) => {
            return GuardianElicitationReview::Decline(
                "guardian MCP elicitation tool_params must be an object",
            );
        }
        None => Some(Value::Object(Map::new())),
    };

    GuardianElicitationReview::ApprovalRequest(Box::new(
        GuardianApprovalRequest::McpToolCall {
            id: format!(
                "mcp_elicitation:{}:{}",
                request.server_name,
                mcp_elicitation_request_id(&request.request_id)
            ),
            server: request.server_name.clone(),
            tool_name,
            arguments,
            connector_id: metadata_owned_string(meta, MCP_ELICITATION_CONNECTOR_ID_KEY),
            connector_name: metadata_owned_string(meta, MCP_ELICITATION_CONNECTOR_NAME_KEY),
            connector_description: metadata_owned_string(
                meta,
                MCP_ELICITATION_CONNECTOR_DESCRIPTION_KEY,
            ),
            tool_title: metadata_owned_string(meta, MCP_ELICITATION_TOOL_TITLE_KEY),
            tool_description: metadata_owned_string(meta, MCP_ELICITATION_TOOL_DESCRIPTION_KEY),
            annotations: None,
        },
    ))
}

pub fn mcp_elicitation_response_from_guardian_decision_parts(
    decision: ReviewDecision,
    denial_message: Option<String>,
) -> ElicitationResponse {
    match decision {
        ReviewDecision::Approved
        | ReviewDecision::ApprovedForSession
        | ReviewDecision::ApprovedExecpolicyAmendment { .. }
        | ReviewDecision::NetworkPolicyAmendment { .. } => ElicitationResponse {
            action: ElicitationAction::Accept,
            content: Some(serde_json::json!({})),
            meta: Some(mcp_elicitation_auto_meta()),
        },
        ReviewDecision::Denied => mcp_elicitation_decline_with_message(
            denial_message.unwrap_or_else(|| "Guardian denied this request.".to_string()),
        ),
        ReviewDecision::TimedOut => mcp_elicitation_decline_with_message(
            denial_message.unwrap_or_else(|| "Guardian timed out reviewing this request.".to_string()),
        ),
        ReviewDecision::Abort => ElicitationResponse {
            action: ElicitationAction::Cancel,
            content: None,
            meta: Some(mcp_elicitation_auto_meta()),
        },
    }
}

pub fn mcp_elicitation_request_id(id: &codex_protocol::mcp::RequestId) -> String {
    match id {
        codex_protocol::mcp::RequestId::String(value) => value.to_string(),
        codex_protocol::mcp::RequestId::Integer(value) => value.to_string(),
    }
}

fn meta_requests_approval_request(meta: &Option<Value>) -> bool {
    meta.as_ref()
        .and_then(Value::as_object)
        .and_then(|meta| metadata_str(meta, MCP_ELICITATION_REQUEST_TYPE_KEY))
        == Some(MCP_ELICITATION_REQUEST_TYPE_APPROVAL_REQUEST)
}

fn requested_schema_has_properties(schema: &Value) -> bool {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| !properties.is_empty())
}

fn metadata_str<'a>(meta: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    meta.get(key).and_then(Value::as_str)
}

fn metadata_owned_string(meta: &Map<String, Value>, key: &str) -> Option<String> {
    metadata_str(meta, key)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn mcp_elicitation_decline_with_message(message: String) -> ElicitationResponse {
    ElicitationResponse {
        action: ElicitationAction::Decline,
        content: None,
        meta: Some(serde_json::json!({
            MCP_ELICITATION_DECLINE_MESSAGE_KEY: message,
            MCP_ELICITATION_APPROVALS_REVIEWER_KEY: ApprovalsReviewer::AutoReview,
        })),
    }
}

fn mcp_elicitation_auto_meta() -> serde_json::Value {
    serde_json::json!({
        MCP_ELICITATION_APPROVALS_REVIEWER_KEY: ApprovalsReviewer::AutoReview,
    })
}
