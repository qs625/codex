//! Turn-scoped mutable state that does not depend on concrete session runtime types.

use std::collections::HashMap;
use std::collections::HashSet;

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

use crate::PendingInputItem;

/// Mutable state for a single turn.
pub struct TurnState {
    pending_approvals: HashMap<String, oneshot::Sender<ReviewDecision>>,
    pending_request_permissions: HashMap<String, PendingRequestPermissions>,
    pending_user_input: HashMap<String, oneshot::Sender<RequestUserInputResponse>>,
    pending_elicitations: HashMap<(String, RequestId), oneshot::Sender<ElicitationResponse>>,
    pending_dynamic_tools: HashMap<String, oneshot::Sender<DynamicToolResponse>>,
    pending_input: Vec<PendingInputItem>,
    client_recoveries: HashMap<String, protocol::models::ResponseItem>,
    client_recoveries_in_current_run: HashSet<String>,
    succeeded_client_recoveries: HashSet<String>,
    client_recovery_retry_wakes: HashSet<String>,
    accepts_async_input_for_current_turn: bool,
    granted_permissions: Option<AdditionalPermissionProfile>,
    strict_auto_review_enabled: bool,
    terminal_handoff: bool,
    pub tool_calls: u64,
    pub has_memory_citation: bool,
    pub token_usage_at_turn_start: TokenUsage,
}

impl Default for TurnState {
    fn default() -> Self {
        Self {
            pending_approvals: HashMap::default(),
            pending_request_permissions: HashMap::default(),
            pending_user_input: HashMap::default(),
            pending_elicitations: HashMap::default(),
            pending_dynamic_tools: HashMap::default(),
            pending_input: Vec::default(),
            client_recoveries: HashMap::default(),
            client_recoveries_in_current_run: HashSet::default(),
            succeeded_client_recoveries: HashSet::default(),
            client_recovery_retry_wakes: HashSet::default(),
            accepts_async_input_for_current_turn: true,
            granted_permissions: None,
            strict_auto_review_enabled: false,
            terminal_handoff: false,
            tool_calls: 0,
            has_memory_citation: false,
            token_usage_at_turn_start: TokenUsage::default(),
        }
    }
}

/// Pending response channel for a model-visible request-permissions call.
pub struct PendingRequestPermissions {
    pub tx_response: oneshot::Sender<RequestPermissionsResponse>,
    pub requested_permissions: RequestPermissionProfile,
    pub cwd: AbsolutePathBuf,
}

impl TurnState {
    pub fn mark_terminal_handoff(&mut self) {
        self.terminal_handoff = true;
    }

    pub fn terminal_handoff(&self) -> bool {
        self.terminal_handoff
    }

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

    pub fn extract_pending_input_matching<F>(&mut self, mut predicate: F) -> Vec<PendingInputItem>
    where
        F: FnMut(&PendingInputItem) -> bool,
    {
        let mut extracted = Vec::new();
        let mut kept = Vec::with_capacity(self.pending_input.len());
        for item in self.pending_input.drain(..) {
            if predicate(&item) {
                extracted.push(item);
            } else {
                kept.push(item);
            }
        }
        self.pending_input = kept;
        extracted
    }

    pub fn has_pending_input(&self) -> bool {
        !self.pending_input.is_empty()
    }

    pub fn record_client_recovery(
        &mut self,
        recovery_id: String,
        response_item: protocol::models::ResponseItem,
    ) {
        self.client_recoveries
            .insert(recovery_id.clone(), response_item);
        if !self.succeeded_client_recoveries.contains(&recovery_id) {
            self.client_recoveries_in_current_run.insert(recovery_id);
        }
    }

    pub fn has_client_recovery(&self, recovery_id: &str) -> bool {
        self.client_recoveries.contains_key(recovery_id)
    }

    pub fn client_recoveries(&self) -> Vec<(protocol::models::ResponseItem, String)> {
        self.client_recoveries
            .iter()
            .map(|(recovery_id, response_item)| (response_item.clone(), recovery_id.clone()))
            .collect()
    }

    pub fn begin_client_recovery_run(&mut self) {
        self.client_recoveries_in_current_run = self
            .client_recoveries
            .keys()
            .filter(|recovery_id| !self.succeeded_client_recoveries.contains(*recovery_id))
            .cloned()
            .collect();
    }

    pub fn mark_client_recovery_run_succeeded(&mut self) {
        self.succeeded_client_recoveries
            .extend(self.client_recoveries_in_current_run.iter().cloned());
    }

    pub fn client_recoveries_by_completion(
        &self,
    ) -> (
        Vec<(protocol::models::ResponseItem, String)>,
        Vec<(protocol::models::ResponseItem, String)>,
    ) {
        self.client_recoveries
            .iter()
            .map(|(recovery_id, response_item)| (response_item.clone(), recovery_id.clone()))
            .partition(|(_, recovery_id)| self.succeeded_client_recoveries.contains(recovery_id))
    }

    pub fn request_client_recovery_retry(&mut self, recovery_id: String) {
        self.client_recovery_retry_wakes.insert(recovery_id);
    }

    pub fn client_recovery_retry_wakes(&self) -> HashSet<String> {
        self.client_recovery_retry_wakes.clone()
    }

    pub fn has_client_recovery_retry_wake(&self, recovery_id: &str) -> bool {
        self.client_recovery_retry_wakes.contains(recovery_id)
    }

    pub fn accept_async_input_for_current_turn(&mut self) {
        self.accepts_async_input_for_current_turn = true;
    }

    pub fn defer_async_input_to_next_turn(&mut self) {
        if self.has_pending_input() {
            return;
        }
        self.accepts_async_input_for_current_turn = false;
    }

    pub fn accepts_async_input_for_current_turn(&self) -> bool {
        self.accepts_async_input_for_current_turn
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
