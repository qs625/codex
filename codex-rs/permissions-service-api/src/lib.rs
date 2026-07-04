pub mod amend;
mod decision;
pub mod error;
pub mod executable_name;
pub mod policy;
mod requirements;
pub mod rule;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use protocol::models::PermissionProfile;
use protocol::models::SandboxPermissions;
use protocol::permissions::FileSystemSandboxPolicy;
use protocol::protocol::AskForApproval;

pub use amend::AmendError;
pub use amend::blocking_append_allow_prefix_rule;
pub use amend::blocking_append_network_rule;
pub use decision::Decision;
pub use error::Error;
pub use error::Result;
pub use policy::Evaluation;
pub use policy::MatchOptions;
pub use policy::Policy;
pub use requirements::RequirementsExecPolicy;
pub use requirements::RequirementsExecPolicyDecisionToml;
pub use requirements::RequirementsExecPolicyParseError;
pub use requirements::RequirementsExecPolicyPatternTokenToml;
pub use requirements::RequirementsExecPolicyPrefixRuleToml;
pub use requirements::RequirementsExecPolicyToml;
pub use requirements::NetworkConstraints;
pub use requirements::NetworkRequirementsToml;
pub use requirements::RemoteSandboxConfigToml;
pub use requirements::SandboxModeRequirement;
pub use requirements::sandbox_mode_requirement_for_permission_profile;
pub use rule::NetworkRuleProtocol;
pub use rule::PatternToken;
pub use rule::PrefixPattern;
pub use rule::PrefixRule;
pub use rule::Rule;
pub use rule::RuleMatch;
pub use rule::RuleRef;

/// Boxed future returned by object-safe permissions service APIs.
pub type PermissionsServiceFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Tool execution approval outcome after combining user approval policy,
/// sandbox policy, and exec policy evaluation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecApprovalRequirement {
    /// No approval required for this tool call.
    Skip {
        /// The first attempt should skip sandboxing when the caller has an
        /// explicit trust signal, such as an exec-policy allow rule.
        bypass_sandbox: bool,
        /// Proposed exec-policy amendment to skip future approvals for similar
        /// commands.
        proposed_execpolicy_amendment: Option<protocol::approvals::ExecPolicyAmendment>,
    },
    /// Approval required for this tool call.
    NeedsApproval {
        reason: Option<String>,
        /// Proposed exec-policy amendment to skip future approvals for similar
        /// commands.
        proposed_execpolicy_amendment: Option<protocol::approvals::ExecPolicyAmendment>,
    },
    /// Execution forbidden for this tool call.
    Forbidden { reason: String },
}

impl ExecApprovalRequirement {
    /// Returns the exec-policy amendment proposed by this requirement, if any.
    pub fn proposed_execpolicy_amendment(
        &self,
    ) -> Option<&protocol::approvals::ExecPolicyAmendment> {
        match self {
            Self::NeedsApproval {
                proposed_execpolicy_amendment: Some(prefix),
                ..
            } => Some(prefix),
            Self::Skip {
                proposed_execpolicy_amendment: Some(prefix),
                ..
            } => Some(prefix),
            Self::Forbidden { .. }
            | Self::NeedsApproval {
                proposed_execpolicy_amendment: None,
                ..
            }
            | Self::Skip {
                proposed_execpolicy_amendment: None,
                ..
            } => None,
        }
    }
}

/// Command approval request evaluated against the current exec policy.
pub struct ExecPolicyApprovalRequest<'a> {
    pub command: &'a [String],
    pub approval_policy: AskForApproval,
    pub permission_profile: PermissionProfile,
    pub file_system_sandbox_policy: &'a FileSystemSandboxPolicy,
    pub sandbox_cwd: &'a Path,
    pub sandbox_permissions: SandboxPermissions,
    pub prefix_rule: Option<Vec<String>>,
}

/// Permissions decision API exposed by the permissions service.
pub trait PermissionsServiceApi: Send + Sync + 'static {
    /// Evaluate one exec-style command against the active exec policy and
    /// current permission context.
    fn create_exec_approval_requirement<'a>(
        &'a self,
        exec_policy: &'a Policy,
        request: ExecPolicyApprovalRequest<'a>,
    ) -> PermissionsServiceFuture<'a, ExecApprovalRequirement>;
}
