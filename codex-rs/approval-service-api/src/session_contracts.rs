use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use hooks_api::PermissionRequestDecision;
use protocol::ThreadId;
use protocol::approvals::ExecPolicyAmendment;
use protocol::approvals::NetworkApprovalContext;
use protocol::approvals::NetworkPolicyAmendment;
use protocol::models::AdditionalPermissionProfile;
use protocol::protocol::FileChange;
use protocol::protocol::GuardianAssessmentDecisionSource;
use protocol::protocol::ReviewDecision;
use thread_service_api::HookToolName;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadSessionCapability;
use thread_service_api::ThreadTurnCapability;
use thread_service_api::UnifiedExecApprovalKey;

/// Boxed future returned by approval session capability traits.
pub type ApprovalSessionFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewRejectionRecord {
    pub rationale: String,
    pub source: GuardianAssessmentDecisionSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewAssessmentRecord {
    pub risk_level: protocol::protocol::GuardianRiskLevel,
    pub user_authorization: protocol::protocol::GuardianUserAuthorization,
    pub outcome: protocol::protocol::GuardianAssessmentOutcome,
    pub rationale: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewRuntimeError {
    PromptBuild { message: String },
    Session { message: String },
    Parse { message: String },
    Timeout,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewRuntimeOutcome {
    Completed(ReviewAssessmentRecord),
    Error(ReviewRuntimeError),
}

#[derive(Debug)]
pub struct ReviewRuntimeResult {
    pub outcome: ReviewRuntimeOutcome,
    pub analytics_result: codex_analytics_api::GuardianReviewAnalyticsResult,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionRequestPayload {
    pub tool_name: HookToolName,
    pub tool_input: serde_json::Value,
}

impl PermissionRequestPayload {
    pub fn bash(command: String, description: Option<String>) -> Self {
        let mut tool_input = serde_json::Map::new();
        tool_input.insert("command".to_string(), serde_json::Value::String(command));
        if let Some(description) = description {
            tool_input.insert(
                "description".to_string(),
                serde_json::Value::String(description),
            );
        }

        Self {
            tool_name: HookToolName::bash(),
            tool_input: serde_json::Value::Object(tool_input),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolPermissionGrants {
    pub session: Option<AdditionalPermissionProfile>,
    pub turn: Option<AdditionalPermissionProfile>,
}

/// Approval-owned session operations implemented by the thread/session owner.
///
/// These methods are session-side state mutations and UI/runtime round-trips
/// required by approval-service. They do not belong on the general thread
/// capability surface consumed by unrelated domains.
pub trait ApprovalSessionCapability: ThreadSessionCapability {
    fn take_review_rejection<'a>(
        &'a self,
        review_id: &'a str,
    ) -> ApprovalSessionFuture<'a, Option<ReviewRejectionRecord>>;

    fn set_review_rejection<'a>(
        &'a self,
        review_id: String,
        rejection: Option<ReviewRejectionRecord>,
    ) -> ApprovalSessionFuture<'a, ()>;

    fn track_review_analytics<'a>(
        &'a self,
        tracking: codex_analytics_api::GuardianReviewTrackContext,
        result: codex_analytics_api::GuardianReviewAnalyticsResult,
        completed_at_ms: u64,
    ) -> ApprovalSessionFuture<'a, ()>;

    fn run_review_session<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        request: serde_json::Value,
        retry_reason: Option<String>,
    ) -> ApprovalSessionFuture<'a, ReviewRuntimeResult>;

    fn record_review_non_rejection<'a>(&'a self, turn_id: &'a str)
    -> ApprovalSessionFuture<'a, ()>;

    fn record_review_rejection<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        turn_id: &'a str,
    ) -> ApprovalSessionFuture<'a, ()>;

    #[allow(clippy::too_many_arguments)]
    fn request_command_approval<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        call_id: String,
        approval_id: Option<String>,
        command: Vec<String>,
        cwd: codex_utils_absolute_path::AbsolutePathBuf,
        reason: Option<String>,
        network_approval_context: Option<NetworkApprovalContext>,
        proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
        additional_permissions: Option<AdditionalPermissionProfile>,
        available_decisions: Option<Vec<ReviewDecision>>,
    ) -> ApprovalSessionFuture<'a, ReviewDecision>;

    fn request_patch_approval<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        call_id: String,
        changes: HashMap<PathBuf, FileChange>,
        reason: Option<String>,
        grant_root: Option<PathBuf>,
    ) -> ApprovalSessionFuture<'a, ReviewDecision>;

    fn cached_approval_decision<'a>(
        &'a self,
        key: String,
    ) -> ApprovalSessionFuture<'a, Option<ReviewDecision>>;

    fn cache_approval_decision<'a>(
        &'a self,
        keys: Vec<String>,
        decision: ReviewDecision,
    ) -> ApprovalSessionFuture<'a, ()>;

    fn record_approval_request_telemetry<'a>(
        &'a self,
        tool_name: &'a str,
        decision: &'a ReviewDecision,
    ) -> ApprovalSessionFuture<'a, ()>;

    fn persist_network_policy_amendment<'a>(
        &'a self,
        amendment: &'a NetworkPolicyAmendment,
        network_approval_context: &'a NetworkApprovalContext,
    ) -> ApprovalSessionFuture<'a, Result<(), String>>;

    fn record_network_policy_amendment_message<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        amendment: &'a NetworkPolicyAmendment,
    ) -> ApprovalSessionFuture<'a, ()>;

    fn run_permission_request_hooks<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        permission_request_run_id: &'a str,
        permission_request: PermissionRequestPayload,
    ) -> ApprovalSessionFuture<'a, Option<PermissionRequestDecision>>;

    fn tool_permission_grants<'a>(&'a self) -> ApprovalSessionFuture<'a, ToolPermissionGrants>;

    #[allow(clippy::too_many_arguments)]
    fn request_unified_exec_approval<'a>(
        &'a self,
        turn: &'a dyn ThreadRuntimeCapability,
        call_id: String,
        command: Vec<String>,
        cwd: codex_utils_absolute_path::AbsolutePathBuf,
        reason: Option<String>,
        sandbox_permissions: protocol::models::SandboxPermissions,
        tty: bool,
        network_approval_context: Option<NetworkApprovalContext>,
        proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
        additional_permissions: Option<AdditionalPermissionProfile>,
        cache_keys: Vec<UnifiedExecApprovalKey>,
    ) -> ApprovalSessionFuture<'a, ReviewDecision>;

    fn strict_auto_review_enabled_for_turn<'a>(&'a self) -> ApprovalSessionFuture<'a, bool>;

    fn active_turn_runtime<'a>(
        &'a self,
    ) -> ApprovalSessionFuture<'a, Option<std::sync::Arc<dyn ThreadRuntimeCapability>>>;

    fn conversation_id(&self) -> ThreadId;
}
