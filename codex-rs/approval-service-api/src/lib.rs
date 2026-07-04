use std::any::Any;
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Weak;

use codex_analytics_api::GuardianApprovalRequestSource;
use permissions_service_api::Decision as ExecPolicyDecision;
use permissions_service_api::NetworkRuleProtocol as ExecPolicyNetworkRuleProtocol;
use codex_guardian::GuardianApprovalRequest;
use codex_guardian::GuardianNetworkAccessTrigger;
use codex_network_proxy_api::BlockedRequestObserver;
use codex_network_proxy_api::NetworkPolicyDecider;
use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::approvals::ExecPolicyAmendment;
use protocol::approvals::NetworkApprovalContext;
use protocol::approvals::NetworkPolicyAmendment;
use protocol::approvals::NetworkPolicyRuleAction;
use protocol::config_types::ApprovalsReviewer;
use protocol::models::AdditionalPermissionProfile;
use protocol::models::SandboxPermissions;
use protocol::protocol::AskForApproval;
use protocol::protocol::FileChange;
use protocol::protocol::ReviewDecision;
use protocol::protocol::SessionSource;
mod session_contracts;
use thread_service_api::NetworkApprovalMode;
use thread_service_api::NetworkApprovalSpec;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ToolRuntimeNetworkApprovalError;

/// Boxed future returned by object-safe approval service APIs.
pub type ApprovalServiceFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type ApprovalServiceFutureStatic<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
pub use session_contracts::ApprovalSessionCapability;
pub use session_contracts::ApprovalSessionFuture;
pub use session_contracts::PermissionRequestPayload;
pub use session_contracts::ReviewAssessmentRecord;
pub use session_contracts::ReviewRejectionRecord;
pub use session_contracts::ReviewRuntimeError;
pub use session_contracts::ReviewRuntimeOutcome;
pub use session_contracts::ReviewRuntimeResult;
pub use session_contracts::ToolPermissionGrants;

pub const GUARDIAN_REVIEWER_NAME: &str = "guardian";
const GUARDIAN_REJECTION_INSTRUCTIONS: &str = concat!(
    "The agent must not attempt to achieve the same outcome via workaround, ",
    "indirect execution, or policy circumvention. ",
    "Proceed only with a materially safer alternative, ",
    "or if the user explicitly approves the action after being informed of the risk. ",
    "Otherwise, stop and request user input.",
);
const GUARDIAN_TIMEOUT_INSTRUCTIONS: &str = concat!(
    "The automatic permission approval review did not finish before its deadline. ",
    "Do not assume the action is unsafe based on the timeout alone. ",
    "You may retry once, or ask the user for guidance or explicit approval.",
);

pub fn routes_approval_to_guardian(
    approval_policy: &AskForApproval,
    approvals_reviewer: ApprovalsReviewer,
) -> bool {
    matches!(
        approval_policy,
        AskForApproval::OnRequest | AskForApproval::Granular(_)
    ) && approvals_reviewer == ApprovalsReviewer::AutoReview
}

pub fn is_guardian_reviewer_source(session_source: &SessionSource) -> bool {
    matches!(
        session_source,
        SessionSource::SubAgent(protocol::protocol::SubAgentSource::Other(name))
            if name == GUARDIAN_REVIEWER_NAME
    )
}

pub fn guardian_rejection_message_from_rationale(rationale: Option<&str>) -> String {
    let rejection = rationale
        .filter(|rationale| !rationale.trim().is_empty())
        .map(|rationale| ReviewRejectionRecord {
            rationale: rationale.to_string(),
            source: protocol::protocol::GuardianAssessmentDecisionSource::Agent,
        })
        .unwrap_or_else(|| ReviewRejectionRecord {
            rationale: "No rationale provided.".to_string(),
            source: protocol::protocol::GuardianAssessmentDecisionSource::Agent,
        });
    match rejection.source {
        protocol::protocol::GuardianAssessmentDecisionSource::Agent => format!(
            "This action was rejected due to unacceptable risk.\nReason: {}\n{}",
            rejection.rationale.trim(),
            GUARDIAN_REJECTION_INSTRUCTIONS
        ),
    }
}

pub fn guardian_timeout_message() -> String {
    GUARDIAN_TIMEOUT_INSTRUCTIONS.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecPolicyNetworkRuleAmendment {
    pub protocol: ExecPolicyNetworkRuleProtocol,
    pub decision: ExecPolicyDecision,
    pub justification: String,
}

pub fn execpolicy_network_rule_amendment(
    amendment: &NetworkPolicyAmendment,
    network_approval_context: &NetworkApprovalContext,
    host: &str,
) -> ExecPolicyNetworkRuleAmendment {
    let protocol = match network_approval_context.protocol {
        protocol::approvals::NetworkApprovalProtocol::Http => ExecPolicyNetworkRuleProtocol::Http,
        protocol::approvals::NetworkApprovalProtocol::Https => ExecPolicyNetworkRuleProtocol::Https,
        protocol::approvals::NetworkApprovalProtocol::Socks5Tcp => {
            ExecPolicyNetworkRuleProtocol::Socks5Tcp
        }
        protocol::approvals::NetworkApprovalProtocol::Socks5Udp => {
            ExecPolicyNetworkRuleProtocol::Socks5Udp
        }
    };
    let (decision, action_verb) = match amendment.action {
        NetworkPolicyRuleAction::Allow => (ExecPolicyDecision::Allow, "Allow"),
        NetworkPolicyRuleAction::Deny => (ExecPolicyDecision::Forbidden, "Deny"),
    };
    let protocol_label = match network_approval_context.protocol {
        protocol::approvals::NetworkApprovalProtocol::Http => "http",
        protocol::approvals::NetworkApprovalProtocol::Https => "https_connect",
        protocol::approvals::NetworkApprovalProtocol::Socks5Tcp => "socks5_tcp",
        protocol::approvals::NetworkApprovalProtocol::Socks5Udp => "socks5_udp",
    };
    let justification = format!("{action_verb} {protocol_label} access to {host}");

    ExecPolicyNetworkRuleAmendment {
        protocol,
        decision,
        justification,
    }
}

/// Session-scoped network approval runtime owned by approval-service.
pub trait SessionNetworkApprovalApi: Send + Sync + 'static {
    fn as_any(&self) -> &(dyn Any + Send + Sync);

    fn sync_session_approved_hosts_to(
        &self,
        other: Arc<dyn SessionNetworkApprovalApi>,
    ) -> ApprovalServiceFuture<'_, ()>;

    fn build_blocked_request_observer(self: Arc<Self>) -> Arc<dyn BlockedRequestObserver>;

    fn build_network_policy_decider(
        self: Arc<Self>,
        session: Arc<tokio::sync::RwLock<Option<Weak<dyn ApprovalSessionCapability>>>>,
    ) -> Arc<dyn NetworkPolicyDecider>;

    fn begin_network_approval(
        self: Arc<Self>,
        turn_id: &str,
        managed_network_active: bool,
        spec: Option<NetworkApprovalSpec<GuardianNetworkAccessTrigger>>,
    ) -> ApprovalServiceFuture<'_, Option<ActiveNetworkApproval>>;

    fn unregister_call(&self, registration_id: String) -> ApprovalServiceFuture<'_, ()>;

    fn finish_call(
        &self,
        registration_id: String,
    ) -> ApprovalServiceFuture<'_, Result<(), ToolRuntimeNetworkApprovalError>>;
}

#[derive(Clone)]
pub struct DeferredNetworkApproval {
    registration_id: String,
    cancellation_token: tokio_util::sync::CancellationToken,
    runtime: Arc<dyn SessionNetworkApprovalApi>,
}

impl DeferredNetworkApproval {
    pub fn new(
        registration_id: String,
        cancellation_token: tokio_util::sync::CancellationToken,
        runtime: Arc<dyn SessionNetworkApprovalApi>,
    ) -> Self {
        Self {
            registration_id,
            cancellation_token,
            runtime,
        }
    }

    pub fn registration_id(&self) -> &str {
        &self.registration_id
    }

    pub fn cancellation_token(&self) -> tokio_util::sync::CancellationToken {
        self.cancellation_token.clone()
    }

    pub async fn finish(&self) -> Result<(), ToolRuntimeNetworkApprovalError> {
        self.runtime.finish_call(self.registration_id.clone()).await
    }
}

pub struct ActiveNetworkApproval {
    registration_id: Option<String>,
    mode: NetworkApprovalMode,
    cancellation_token: tokio_util::sync::CancellationToken,
    runtime: Arc<dyn SessionNetworkApprovalApi>,
}

impl ActiveNetworkApproval {
    pub fn new(
        registration_id: Option<String>,
        mode: NetworkApprovalMode,
        cancellation_token: tokio_util::sync::CancellationToken,
        runtime: Arc<dyn SessionNetworkApprovalApi>,
    ) -> Self {
        Self {
            registration_id,
            mode,
            cancellation_token,
            runtime,
        }
    }

    pub fn registration_id(&self) -> Option<&str> {
        self.registration_id.as_deref()
    }

    pub fn mode(&self) -> NetworkApprovalMode {
        self.mode
    }

    pub fn cancellation_token(&self) -> tokio_util::sync::CancellationToken {
        self.cancellation_token.clone()
    }

    pub fn into_deferred(self) -> Option<DeferredNetworkApproval> {
        let Self {
            registration_id,
            mode,
            cancellation_token,
            runtime,
        } = self;
        match (mode, registration_id) {
            (NetworkApprovalMode::Deferred, Some(registration_id)) => Some(
                DeferredNetworkApproval::new(registration_id, cancellation_token, runtime),
            ),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecCommandApprovalOutcome {
    ContinueInRuntime,
    Preapproved,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize)]
pub struct ApplyPatchApprovalKey {
    pub environment_id: String,
    pub path: AbsolutePathBuf,
}

/// Routed apply-patch approval payload owned by approval-service.
pub struct ApplyPatchApprovalRequest {
    pub cwd: AbsolutePathBuf,
    pub files: Vec<AbsolutePathBuf>,
    pub patch: String,
}

/// Apply-patch approval request routed through the approval service.
pub struct ApplyPatchApprovalDispatch {
    pub session: Arc<dyn ApprovalSessionCapability>,
    pub turn: Arc<dyn ThreadRuntimeCapability>,
    pub call_id: String,
    pub approval_keys: Vec<ApplyPatchApprovalKey>,
    pub approval_request: ApplyPatchApprovalRequest,
    pub changes: HashMap<PathBuf, FileChange>,
    pub permissions_preapproved: bool,
    pub retry_reason: Option<String>,
}

pub struct ExecCommandApprovalDispatch {
    pub session: Arc<dyn ApprovalSessionCapability>,
    pub turn: Arc<dyn ThreadRuntimeCapability>,
    pub call_id: String,
    pub command: Vec<String>,
    pub hook_command: String,
    pub cwd: std::path::PathBuf,
    pub reason: Option<String>,
    pub justification: Option<String>,
    pub sandbox_permissions: SandboxPermissions,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
    pub tty: bool,
    pub exec_approval_requirement: ExecCommandApprovalRequirement,
    pub approval_keys: Vec<ExecCommandApprovalKey>,
    pub network_approval_context: Option<NetworkApprovalContext>,
}

pub struct GuardianReviewDispatch {
    pub session: Arc<dyn ApprovalSessionCapability>,
    pub turn: Arc<dyn ThreadRuntimeCapability>,
    pub review_id: String,
    pub request: GuardianApprovalRequest,
    pub retry_reason: Option<String>,
    pub approval_request_source: GuardianApprovalRequestSource,
    pub cancellation_token: Option<tokio_util::sync::CancellationToken>,
}

pub struct GuardianReviewResult {
    pub decision: ReviewDecision,
    pub decline_message: Option<String>,
}

#[derive(Clone, Debug)]
pub enum ExecCommandApprovalRequirement {
    Skip {
        bypass_sandbox: bool,
        proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
    },
    NeedsApproval {
        reason: Option<String>,
        proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
    },
    Forbidden {
        reason: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize)]
pub struct ExecCommandApprovalKey {
    pub command: Vec<String>,
    pub cwd: AbsolutePathBuf,
    pub tty: bool,
    pub sandbox_permissions: SandboxPermissions,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
}

/// Global approval service API.
///
/// Tool and other domain services should depend on this trait instead of
/// reaching into thread/session runtime types for approval orchestration.
pub trait ApprovalServiceApi: Send + Sync + 'static {
    fn create_session_network_approval(&self) -> Arc<dyn SessionNetworkApprovalApi>;

    fn request_apply_patch_approval(
        &self,
        request: ApplyPatchApprovalDispatch,
    ) -> ApprovalServiceFuture<'_, Result<(), String>>;

    fn request_exec_command_approval(
        &self,
        request: ExecCommandApprovalDispatch,
    ) -> ApprovalServiceFuture<'_, Result<ExecCommandApprovalOutcome, String>>;

    fn review_guardian_request(
        &self,
        request: GuardianReviewDispatch,
    ) -> ApprovalServiceFuture<'_, GuardianReviewResult>;
}
