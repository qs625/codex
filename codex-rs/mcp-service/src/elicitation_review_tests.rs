use crate::GuardianElicitationReview;
use crate::guardian_elicitation_review_request;
use crate::mcp_elicitation_response_from_guardian_decision_parts;
use mcp_types::ElicitationAction;
use mcp_types::ElicitationResponse;
use mcp_types::ElicitationReviewRequest;
use protocol::config_types::ApprovalsReviewer;
use protocol::mcp::RequestId;
use protocol::protocol::ReviewDecision;
use serde_json::Value;
use serde_json::json;

const GUARDIAN_TIMEOUT_MESSAGE: &str = "Guardian timed out reviewing this request.";

fn meta(value: Value) -> Option<Value> {
    let Value::Object(map) = value else {
        panic!("metadata must be an object");
    };
    Some(Value::Object(map))
}

fn guardian_meta(tool_params: Option<Value>) -> Option<Value> {
    let mut value = json!({
        "codex_approval_kind": "mcp_tool_call",
        "codex_request_type": "approval_request",
        "connector_id": "browser-use",
        "connector_name": "Browser Use",
        "tool_name": "access_browser_origin",
        "tool_title": "Access browser origin",
    });
    if let Some(tool_params) = tool_params {
        value["tool_params"] = tool_params;
    }
    meta(value)
}

fn empty_form_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
    })
}

fn form_request(meta: Option<Value>) -> ElicitationReviewRequest {
    ElicitationReviewRequest {
        server_name: "browser-use".to_string(),
        request_id: RequestId::Integer(7),
        elicitation: protocol::approvals::ElicitationRequest::Form {
            meta,
            message: "Allow origin?".to_string(),
            requested_schema: empty_form_schema(),
        },
    }
}

#[test]
fn guardian_elicitation_review_request_builds_mcp_tool_call() {
    let request = form_request(guardian_meta(Some(json!({
        "origin": "https://example.com",
    }))));

    let GuardianElicitationReview::ApprovalRequest(guardian_request) =
        guardian_elicitation_review_request(&request)
    else {
        panic!("expected Guardian MCP tool call request");
    };
    let codex_guardian::GuardianApprovalRequest::McpToolCall {
        id,
        server,
        tool_name,
        arguments,
        connector_id,
        connector_name,
        connector_description,
        tool_title,
        tool_description,
        annotations,
    } = *guardian_request
    else {
        panic!("expected Guardian MCP tool call request");
    };

    assert_eq!(id, "mcp_elicitation:browser-use:7");
    assert_eq!(server, "browser-use");
    assert_eq!(tool_name, "access_browser_origin");
    assert_eq!(arguments, Some(json!({ "origin": "https://example.com" })));
    assert_eq!(connector_id.as_deref(), Some("browser-use"));
    assert_eq!(connector_name.as_deref(), Some("Browser Use"));
    assert_eq!(connector_description, None);
    assert_eq!(tool_title.as_deref(), Some("Access browser origin"));
    assert_eq!(tool_description, None);
    assert_eq!(annotations, None);
}

#[test]
fn guardian_elicitation_review_request_defaults_missing_tool_params() {
    let request = form_request(guardian_meta(/*tool_params*/ None));

    let GuardianElicitationReview::ApprovalRequest(guardian_request) =
        guardian_elicitation_review_request(&request)
    else {
        panic!("expected Guardian MCP tool call request");
    };
    let codex_guardian::GuardianApprovalRequest::McpToolCall { arguments, .. } = *guardian_request
    else {
        panic!("expected Guardian MCP tool call request");
    };

    assert_eq!(arguments, Some(json!({})));
}

#[test]
fn guardian_elicitation_review_request_requires_opt_in() {
    let request = form_request(meta(json!({
        "codex_approval_kind": "mcp_tool_call",
        "tool_name": "access_browser_origin",
    })));

    assert_eq!(
        guardian_elicitation_review_request(&request),
        GuardianElicitationReview::NotRequested
    );
}

#[test]
fn guardian_elicitation_review_request_declines_unsupported_opt_in_shapes() {
    let url_request = ElicitationReviewRequest {
        server_name: "browser-use".to_string(),
        request_id: RequestId::Integer(8),
        elicitation: protocol::approvals::ElicitationRequest::Url {
            meta: guardian_meta(Some(json!({}))),
            message: "Open URL".to_string(),
            url: "https://example.com".to_string(),
            elicitation_id: "elicit-1".to_string(),
        },
    };
    assert!(matches!(
        guardian_elicitation_review_request(&url_request),
        GuardianElicitationReview::Decline(_)
    ));

    let non_empty_schema_request = ElicitationReviewRequest {
        server_name: "browser-use".to_string(),
        request_id: RequestId::Integer(9),
        elicitation: protocol::approvals::ElicitationRequest::Form {
            meta: guardian_meta(Some(json!({}))),
            message: "Allow origin?".to_string(),
            requested_schema: json!({
                "type": "object",
                "properties": {
                    "confirmed": {
                        "type": "boolean",
                    },
                },
            }),
        },
    };
    assert!(matches!(
        guardian_elicitation_review_request(&non_empty_schema_request),
        GuardianElicitationReview::Decline(_)
    ));

    let missing_tool_name_request = form_request(meta(json!({
        "codex_approval_kind": "mcp_tool_call",
        "codex_request_type": "approval_request",
    })));
    assert!(matches!(
        guardian_elicitation_review_request(&missing_tool_name_request),
        GuardianElicitationReview::Decline(_)
    ));
}

#[test]
fn guardian_decisions_map_to_elicitation_responses_without_session_state() {
    assert_eq!(
        mcp_elicitation_response_from_guardian_decision_parts(
            ReviewDecision::Approved,
            /*denial_message*/ None,
        ),
        ElicitationResponse {
            action: ElicitationAction::Accept,
            content: Some(json!({})),
            meta: Some(json!({
                "approvals_reviewer": ApprovalsReviewer::AutoReview,
            })),
        }
    );
    assert_eq!(
        mcp_elicitation_response_from_guardian_decision_parts(
            ReviewDecision::Denied,
            Some("Denied by Guardian".to_string()),
        ),
        ElicitationResponse {
            action: ElicitationAction::Decline,
            content: None,
            meta: Some(json!({
                "approvals_reviewer": ApprovalsReviewer::AutoReview,
                "message": "Denied by Guardian",
            })),
        }
    );
    assert_eq!(
        mcp_elicitation_response_from_guardian_decision_parts(
            ReviewDecision::TimedOut,
            /*denial_message*/ None,
        ),
        ElicitationResponse {
            action: ElicitationAction::Decline,
            content: None,
            meta: Some(json!({
                "approvals_reviewer": ApprovalsReviewer::AutoReview,
                "message": GUARDIAN_TIMEOUT_MESSAGE,
            })),
        }
    );
    assert_eq!(
        mcp_elicitation_response_from_guardian_decision_parts(
            ReviewDecision::Abort,
            /*denial_message*/ None,
        ),
        ElicitationResponse {
            action: ElicitationAction::Cancel,
            content: None,
            meta: Some(json!({
                "approvals_reviewer": ApprovalsReviewer::AutoReview,
            })),
        }
    );
}
