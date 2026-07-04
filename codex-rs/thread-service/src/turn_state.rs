//! Turn-scoped mutable state that does not depend on concrete session runtime types.

use std::collections::HashMap;

use codex_sandboxing_api::policy_transforms::merge_permission_profiles;
use codex_utils_absolute_path::AbsolutePathBuf;
use mcp_types::ElicitationResponse;
use protocol::dynamic_tools::DynamicToolResponse;
use protocol::mcp::RequestId;
use protocol::models::AdditionalPermissionProfile;
use protocol::protocol::ReviewDecision;
use protocol::protocol::TokenUsage;
use protocol::request_permissions::RequestPermissionProfile;
use protocol::request_permissions::RequestPermissionsResponse;
use protocol::request_user_input::RequestUserInputResponse;
use tokio::sync::oneshot;

use crate::MailboxDeliveryPhase;
use crate::PendingInputItem;

/// Mutable state for a single turn.
#[derive(Default)]
pub struct TurnState {
    pending_approvals: HashMap<String, oneshot::Sender<ReviewDecision>>,
    pending_request_permissions: HashMap<String, PendingRequestPermissions>,
    pending_user_input: HashMap<String, oneshot::Sender<RequestUserInputResponse>>,
    pending_elicitations: HashMap<(String, RequestId), oneshot::Sender<ElicitationResponse>>,
    pending_dynamic_tools: HashMap<String, oneshot::Sender<DynamicToolResponse>>,
    pending_input: Vec<PendingInputItem>,
    mailbox_delivery_phase: MailboxDeliveryPhase,
    granted_permissions: Option<AdditionalPermissionProfile>,
    strict_auto_review_enabled: bool,
    pub tool_calls: u64,
    pub has_memory_citation: bool,
    pub token_usage_at_turn_start: TokenUsage,
}

/// Pending response channel for a model-visible request-permissions call.
pub struct PendingRequestPermissions {
    pub tx_response: oneshot::Sender<RequestPermissionsResponse>,
    pub requested_permissions: RequestPermissionProfile,
    pub cwd: AbsolutePathBuf,
}

impl TurnState {
    pub fn insert_pending_approval(
        &mut self,
        key: String,
        tx: oneshot::Sender<ReviewDecision>,
    ) -> Option<oneshot::Sender<ReviewDecision>> {
        self.pending_approvals.insert(key, tx)
    }

    pub fn remove_pending_approval(
        &mut self,
        key: &str,
    ) -> Option<oneshot::Sender<ReviewDecision>> {
        self.pending_approvals.remove(key)
    }

    pub fn clear_pending(&mut self) {
        self.pending_approvals.clear();
        self.pending_request_permissions.clear();
        self.pending_user_input.clear();
        self.pending_elicitations.clear();
        self.pending_dynamic_tools.clear();
        self.pending_input.clear();
    }

    pub fn insert_pending_request_permissions(
        &mut self,
        key: String,
        pending_request_permissions: PendingRequestPermissions,
    ) -> Option<PendingRequestPermissions> {
        self.pending_request_permissions
            .insert(key, pending_request_permissions)
    }

    pub fn remove_pending_request_permissions(
        &mut self,
        key: &str,
    ) -> Option<PendingRequestPermissions> {
        self.pending_request_permissions.remove(key)
    }

    pub fn insert_pending_user_input(
        &mut self,
        key: String,
        tx: oneshot::Sender<RequestUserInputResponse>,
    ) -> Option<oneshot::Sender<RequestUserInputResponse>> {
        self.pending_user_input.insert(key, tx)
    }

    pub fn remove_pending_user_input(
        &mut self,
        key: &str,
    ) -> Option<oneshot::Sender<RequestUserInputResponse>> {
        self.pending_user_input.remove(key)
    }

    pub fn insert_pending_elicitation(
        &mut self,
        server_name: String,
        request_id: RequestId,
        tx: oneshot::Sender<ElicitationResponse>,
    ) -> Option<oneshot::Sender<ElicitationResponse>> {
        self.pending_elicitations
            .insert((server_name, request_id), tx)
    }

    pub fn remove_pending_elicitation(
        &mut self,
        server_name: &str,
        request_id: &RequestId,
    ) -> Option<oneshot::Sender<ElicitationResponse>> {
        self.pending_elicitations
            .remove(&(server_name.to_string(), request_id.clone()))
    }

    pub fn insert_pending_dynamic_tool(
        &mut self,
        key: String,
        tx: oneshot::Sender<DynamicToolResponse>,
    ) -> Option<oneshot::Sender<DynamicToolResponse>> {
        self.pending_dynamic_tools.insert(key, tx)
    }

    pub fn remove_pending_dynamic_tool(
        &mut self,
        key: &str,
    ) -> Option<oneshot::Sender<DynamicToolResponse>> {
        self.pending_dynamic_tools.remove(key)
    }

    pub fn push_pending_input(&mut self, input: PendingInputItem) {
        self.pending_input.push(input);
    }

    pub fn prepend_pending_input(&mut self, mut input: Vec<PendingInputItem>) {
        if input.is_empty() {
            return;
        }

        input.append(&mut self.pending_input);
        self.pending_input = input;
    }

    pub fn take_pending_input(&mut self) -> Vec<PendingInputItem> {
        if self.pending_input.is_empty() {
            Vec::with_capacity(0)
        } else {
            let mut ret = Vec::new();
            std::mem::swap(&mut ret, &mut self.pending_input);
            ret
        }
    }

    pub fn pending_input(&self) -> &[PendingInputItem] {
        &self.pending_input
    }

    pub fn has_pending_input(&self) -> bool {
        !self.pending_input.is_empty()
    }

    pub fn accept_mailbox_delivery_for_current_turn(&mut self) {
        self.set_mailbox_delivery_phase(MailboxDeliveryPhase::CurrentTurn);
    }

    pub fn accepts_mailbox_delivery_for_current_turn(&self) -> bool {
        self.mailbox_delivery_phase == MailboxDeliveryPhase::CurrentTurn
    }

    pub fn set_mailbox_delivery_phase(&mut self, phase: MailboxDeliveryPhase) {
        self.mailbox_delivery_phase = phase;
    }

    pub fn record_granted_permissions(&mut self, permissions: AdditionalPermissionProfile) {
        self.granted_permissions =
            merge_permission_profiles(self.granted_permissions.as_ref(), Some(&permissions));
    }

    pub fn granted_permissions(&self) -> Option<AdditionalPermissionProfile> {
        self.granted_permissions.clone()
    }

    pub fn enable_strict_auto_review(&mut self) {
        self.strict_auto_review_enabled = true;
    }

    pub fn strict_auto_review_enabled(&self) -> bool {
        self.strict_auto_review_enabled
    }
}
