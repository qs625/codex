use crate::HookToolName;
use crate::NetworkApprovalSpec;
use crate::ToolError;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
pub use codex_permissions_runtime::ExecApprovalRequirement;
pub use codex_permissions_runtime::default_exec_approval_requirement;
use codex_protocol::approvals::NetworkApprovalContext;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::ReviewDecision;
use codex_sandboxing_api::SandboxRuntime;
use codex_sandboxing_api::SandboxType;
use codex_sandboxing_api::SandboxablePreference;
use codex_tool_planning::ToolName;
use codex_utils_absolute_path::AbsolutePathBuf;
use futures::future::BoxFuture;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Default, Debug)]
pub struct ApprovalStore {
    // Store serialized keys for generic caching across requests.
    map: HashMap<String, ReviewDecision>,
}

impl ApprovalStore {
    pub fn get<K>(&self, key: &K) -> Option<ReviewDecision>
    where
        K: Serialize,
    {
        let s = serde_json::to_string(key).ok()?;
        self.map.get(&s).cloned()
    }

    pub fn put<K>(&mut self, key: K, value: ReviewDecision)
    where
        K: Serialize,
    {
        if let Ok(s) = serde_json::to_string(&key) {
            self.map.insert(s, value);
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxOverride {
    NoOverride,
    BypassSandboxFirstAttempt,
}

pub fn sandbox_override_for_first_attempt(
    sandbox_permissions: SandboxPermissions,
    exec_approval_requirement: &ExecApprovalRequirement,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
) -> SandboxOverride {
    // ExecPolicy `Allow` can intentionally imply full trust (Skip + bypass_sandbox=true),
    // which supersedes `with_additional_permissions` sandboxed execution hints.
    if matches!(
        exec_approval_requirement,
        ExecApprovalRequirement::Skip {
            bypass_sandbox: true,
            ..
        }
    ) {
        return SandboxOverride::BypassSandboxFirstAttempt;
    }

    // Deny-read restrictions suppress explicit escalation because that path
    // would otherwise discard the filesystem policy entirely.
    if file_system_sandbox_policy.has_denied_read_restrictions() {
        return SandboxOverride::NoOverride;
    }

    if sandbox_permissions.requires_escalated_permissions() {
        SandboxOverride::BypassSandboxFirstAttempt
    } else {
        SandboxOverride::NoOverride
    }
}

pub fn managed_network_for_sandbox_permissions(
    network: Option<&SharedNetworkProxyRuntime>,
    sandbox_permissions: SandboxPermissions,
) -> Option<SharedNetworkProxyRuntime> {
    if sandbox_permissions.requires_escalated_permissions() {
        None
    } else {
        network.cloned()
    }
}

pub fn should_bypass_approval(policy: AskForApproval, already_approved: bool) -> bool {
    if already_approved {
        // We do not ask one more time.
        return true;
    }
    matches!(policy, AskForApproval::Never)
}

pub fn wants_no_sandbox_approval(policy: AskForApproval) -> bool {
    match policy {
        AskForApproval::OnFailure => true,
        AskForApproval::UnlessTrusted => true,
        AskForApproval::Never => false,
        AskForApproval::OnRequest => false,
        AskForApproval::Granular(granular_config) => granular_config.sandbox_approval,
    }
}

#[derive(Clone)]
pub struct ApprovalCtx<'a, Session, Turn> {
    pub session: &'a Session,
    pub turn: &'a Turn,
    pub call_id: &'a str,
    /// Guardian review lifecycle ID for this approval, when guardian is reviewing it.
    ///
    /// This is separate from `call_id`: `call_id` identifies the tool item under
    /// review, while this ID identifies the review itself. Keeping both lets
    /// denial handling, overrides, and app-server notifications refer to the
    /// review without overloading the tool call ID as a review ID.
    pub guardian_review_id: Option<String>,
    pub retry_reason: Option<String>,
    pub network_approval_context: Option<NetworkApprovalContext>,
}

/// Approval contract implemented by a tool runtime request type.
///
/// Implementations expose cache keys, optional hook payloads, and the host
/// approval entrypoint while leaving the concrete UI/guardian flow to the host.
pub trait Approvable<Req> {
    type Session;
    type Turn;
    type ApprovalKey: Hash + Eq + Clone + Debug + Serialize;

    // In most cases (shell, unified_exec), a request will have a single approval key.
    //
    // However, apply_patch needs session "Allow, don't ask again" semantics that
    // apply to multiple atomic targets (e.g., apply_patch approves per file path). Returning
    // a list of keys lets the runtime treat the request as approved-for-session only if
    // *all* keys are already approved, while still caching approvals per-key so future
    // requests touching any subset can also skip prompting.
    fn approval_keys(&self, req: &Req) -> Vec<Self::ApprovalKey>;

    /// Return per-request sandbox permissions for first-attempt sandbox
    /// selection. Most tools use the ambient sandbox policy unchanged.
    fn sandbox_permissions(&self, _req: &Req) -> SandboxPermissions {
        SandboxPermissions::UseDefault
    }

    fn approval_preapproved(&self, _req: &Req) -> bool {
        false
    }

    fn should_bypass_approval(&self, policy: AskForApproval, already_approved: bool) -> bool {
        should_bypass_approval(policy, already_approved)
    }

    /// Return `Some(_)` to specify a custom exec approval requirement, or `None`
    /// to fall back to policy-based default.
    fn exec_approval_requirement(&self, _req: &Req) -> Option<ExecApprovalRequirement> {
        None
    }

    /// Return hook input for approval-time policy hooks when this runtime wants
    /// hook evaluation to run before guardian or user approval.
    fn permission_request_payload(&self, _req: &Req) -> Option<PermissionRequestPayload> {
        None
    }

    /// Decide we can request an approval for no-sandbox execution.
    fn wants_no_sandbox_approval(&self, policy: AskForApproval) -> bool {
        wants_no_sandbox_approval(policy)
    }

    fn start_approval_async<'a>(
        &'a mut self,
        req: &'a Req,
        ctx: ApprovalCtx<'a, Self::Session, Self::Turn>,
    ) -> BoxFuture<'a, ReviewDecision>;
}

/// Sandbox preference contract implemented by concrete tool runtimes.
pub trait Sandboxable {
    fn sandbox_preference(&self) -> SandboxablePreference;

    fn escalate_on_failure(&self) -> bool {
        true
    }
}

pub struct ToolCtx<Session, Turn> {
    pub session: Session,
    pub turn: Turn,
    pub call_id: String,
    pub tool_name: ToolName,
}

/// Host-neutral execution contract implemented by a concrete tool runtime.
///
/// The API crate owns the contract so session/core code can depend on the
/// interface without depending on a specific `codex-tool-runtime`
/// implementation module.
pub trait ToolRuntime<Req, Out>: Approvable<Req> + Sandboxable {
    type NetworkApprovalTrigger;

    fn network_approval_spec(
        &self,
        _req: &Req,
        _ctx: &ToolCtx<Self::Session, Self::Turn>,
    ) -> Option<NetworkApprovalSpec<Self::NetworkApprovalTrigger>> {
        None
    }

    fn sandbox_cwd<'a>(&self, _req: &'a Req) -> Option<&'a AbsolutePathBuf> {
        None
    }

    fn run(
        &mut self,
        req: &Req,
        attempt: &SandboxAttempt<'_>,
        ctx: &ToolCtx<Self::Session, Self::Turn>,
    ) -> impl std::future::Future<Output = Result<Out, ToolError>> + Send;
}

pub struct SandboxAttempt<'a> {
    pub sandbox: SandboxType,
    pub permissions: &'a PermissionProfile,
    pub enforce_managed_network: bool,
    pub sandbox_runtime: &'a dyn SandboxRuntime,
    pub sandbox_cwd: &'a AbsolutePathBuf,
    pub codex_linux_sandbox_exe: Option<&'a PathBuf>,
    pub use_legacy_landlock: bool,
    pub windows_sandbox_level: WindowsSandboxLevel,
    pub windows_sandbox_private_desktop: bool,
    pub network_denial_cancellation_token: Option<CancellationToken>,
}
