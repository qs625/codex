//! MCP elicitation request tracking and policy handling.
//!
//! RMCP clients call into this module when a server asks Codex to elicit data
//! from the user. It decides whether the request can be automatically accepted,
//! must be declined by policy, or should be surfaced as a Codex protocol event
//! and later resolved through the stored responder.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use crate::mcp::McpPermissionPromptAutoApproveContext;
use crate::mcp::mcp_permission_prompt_is_auto_approved;
use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use async_channel::Sender;
use codex_rmcp_client::SendElicitation;
use futures::future::FutureExt;
use mcp_types::ElicitationAction;
use mcp_types::ElicitationResponse;
use mcp_types::ElicitationReviewRequest;
use mcp_types::ElicitationReviewerHandle;
use protocol::approvals::ElicitationRequest;
use protocol::approvals::ElicitationRequestEvent;
use protocol::mcp::RequestId as ProtocolRequestId;
use protocol::models::PermissionProfile;
use protocol::protocol::AskForApproval;
use protocol::protocol::Event;
use protocol::protocol::EventMsg;
use rmcp::model::CreateElicitationRequestParams;
use rmcp::model::RequestId as RmcpRequestId;
use tokio::sync::Mutex;
use tokio::sync::oneshot;

#[derive(Clone)]
pub(crate) struct ElicitationRequestManager {
    requests: Arc<Mutex<ResponderMap>>,
    pub(crate) approval_policy: Arc<StdMutex<AskForApproval>>,
    pub(crate) permission_profile: Arc<StdMutex<PermissionProfile>>,
    auto_deny: Arc<StdMutex<bool>>,
    reviewer: Option<ElicitationReviewerHandle>,
}

impl ElicitationRequestManager {
    pub(crate) fn new(
        approval_policy: AskForApproval,
        permission_profile: PermissionProfile,
        reviewer: Option<ElicitationReviewerHandle>,
    ) -> Self {
        Self {
            requests: Arc::new(Mutex::new(HashMap::new())),
            approval_policy: Arc::new(StdMutex::new(approval_policy)),
            permission_profile: Arc::new(StdMutex::new(permission_profile)),
            auto_deny: Arc::new(StdMutex::new(false)),
            reviewer,
        }
    }

    pub(crate) fn auto_deny(&self) -> bool {
        self.auto_deny
            .lock()
            .map(|auto_deny| *auto_deny)
            .unwrap_or(false)
    }

    pub(crate) fn set_auto_deny(&self, auto_deny: bool) {
        if let Ok(mut current) = self.auto_deny.lock() {
            *current = auto_deny;
        }
    }

    pub(crate) async fn resolve(
        &self,
        server_name: String,
        id: ProtocolRequestId,
        response: ElicitationResponse,
    ) -> Result<()> {
        self.requests
            .lock()
            .await
            .remove(&(server_name, id))
            .ok_or_else(|| anyhow!("elicitation request not found"))?
            .send(response)
            .map_err(|e| anyhow!("failed to send elicitation response: {e:?}"))
    }

    pub(crate) fn make_sender(
        &self,
        server_name: String,
        tx_event: Sender<Event>,
    ) -> SendElicitation {
        let elicitation_requests = self.requests.clone();
        let approval_policy = self.approval_policy.clone();
        let permission_profile = self.permission_profile.clone();
        let auto_deny = self.auto_deny.clone();
        let reviewer = self.reviewer.clone();
        Box::new(move |id, elicitation| {
            let elicitation_requests = elicitation_requests.clone();
            let tx_event = tx_event.clone();
            let server_name = server_name.clone();
            let approval_policy = approval_policy.clone();
            let permission_profile = permission_profile.clone();
            let auto_deny = auto_deny.clone();
            let reviewer = reviewer.clone();
            async move {
                let auto_deny = auto_deny
                    .lock()
                    .map(|auto_deny| *auto_deny)
                    .unwrap_or(false);
                if auto_deny {
                    return Ok(ElicitationResponse {
                        action: ElicitationAction::Decline,
                        content: None,
                        meta: None,
                    });
                }

                let approval_policy = approval_policy
                    .lock()
                    .map(|policy| *policy)
                    .unwrap_or(AskForApproval::Never);
                let permission_profile = permission_profile
                    .lock()
                    .map(|profile| profile.clone())
                    .unwrap_or_default();
                if mcp_permission_prompt_is_auto_approved(
                    approval_policy,
                    &permission_profile,
                    McpPermissionPromptAutoApproveContext::default(),
                ) && can_auto_accept_elicitation(&elicitation)
                {
                    return Ok(ElicitationResponse {
                        action: ElicitationAction::Accept,
                        content: Some(serde_json::json!({})),
                        meta: None,
                    });
                }

                if elicitation_is_rejected_by_policy(approval_policy) {
                    return Ok(ElicitationResponse {
                        action: ElicitationAction::Decline,
                        content: None,
                        meta: None,
                    });
                }

                let protocol_id = protocol_request_id(&id);

                if let Some(reviewer) = reviewer.as_ref() {
                    let request = ElicitationReviewRequest {
                        server_name: server_name.clone(),
                        request_id: protocol_id.clone(),
                        elicitation: protocol_elicitation_request(&elicitation)?,
                    };
                    if let Some(response) = reviewer.review(request).await? {
                        return Ok(response);
                    }
                }

                let request = protocol_elicitation_request(&elicitation)?;
                let (tx, rx) = oneshot::channel();
                {
                    let mut lock = elicitation_requests.lock().await;
                    lock.insert((server_name.clone(), protocol_id.clone()), tx);
                }
                let _ = tx_event
                    .send(Event {
                        id: "mcp_elicitation_request".to_string(),
                        msg: EventMsg::ElicitationRequest(ElicitationRequestEvent {
                            turn_id: None,
                            server_name,
                            id: protocol_id,
                            request,
                        }),
                    })
                    .await;
                rx.await
                    .context("elicitation request channel closed unexpectedly")
            }
            .boxed()
        })
    }
}

pub(crate) fn elicitation_is_rejected_by_policy(approval_policy: AskForApproval) -> bool {
    match approval_policy {
        AskForApproval::Never => true,
        AskForApproval::OnFailure => false,
        AskForApproval::OnRequest => false,
        AskForApproval::UnlessTrusted => false,
        AskForApproval::Granular(granular_config) => !granular_config.allows_mcp_elicitations(),
    }
}

type ResponderMap = HashMap<(String, ProtocolRequestId), oneshot::Sender<ElicitationResponse>>;

fn protocol_request_id(id: &RmcpRequestId) -> ProtocolRequestId {
    match id {
        rmcp::model::NumberOrString::String(value) => ProtocolRequestId::String(value.to_string()),
        rmcp::model::NumberOrString::Number(value) => ProtocolRequestId::Integer(*value),
    }
}

fn protocol_elicitation_request(
    elicitation: &CreateElicitationRequestParams,
) -> Result<ElicitationRequest> {
    match elicitation {
        CreateElicitationRequestParams::FormElicitationParams {
            meta,
            message,
            requested_schema,
        } => Ok(ElicitationRequest::Form {
            meta: meta
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .context("failed to serialize MCP elicitation metadata")?,
            message: message.clone(),
            requested_schema: serde_json::to_value(requested_schema)
                .context("failed to serialize MCP elicitation schema")?,
        }),
        CreateElicitationRequestParams::UrlElicitationParams {
            meta,
            message,
            url,
            elicitation_id,
        } => Ok(ElicitationRequest::Url {
            meta: meta
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .context("failed to serialize MCP elicitation metadata")?,
            message: message.clone(),
            url: url.clone(),
            elicitation_id: elicitation_id.clone(),
        }),
    }
}

fn can_auto_accept_elicitation(elicitation: &CreateElicitationRequestParams) -> bool {
    match elicitation {
        CreateElicitationRequestParams::FormElicitationParams {
            requested_schema, ..
        } => {
            // Auto-accept confirm/approval elicitations without schema requirements.
            requested_schema.properties.is_empty()
        }
        CreateElicitationRequestParams::UrlElicitationParams { .. } => false,
    }
}
