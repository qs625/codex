//! Shared approvals and sandboxing traits used by tool runtimes.
//!
//! Consolidates the approval flow primitives (`ApprovalDecision`, `ApprovalStore`,
//! `ApprovalCtx`, `Approvable`) together with the sandbox orchestration traits
//! and helpers (`Sandboxable`, `ToolRuntime`, `SandboxAttempt`, etc.).

use crate::sandboxing::ExecOptions;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::state::SessionServices;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_protocol::protocol::ReviewDecision;
use codex_sandboxing_api::SandboxCommand;
use codex_sandboxing_api::SandboxTransformError;
use codex_sandboxing_api::SandboxTransformRequest;
#[cfg(test)]
pub(crate) use codex_tool_runtime_api::Approvable;
pub(crate) type ApprovalCtx<'a> =
    codex_tool_runtime_api::ApprovalCtx<'a, std::sync::Arc<Session>, std::sync::Arc<TurnContext>>;
#[cfg(test)]
pub(crate) use codex_tool_runtime_api::ExecApprovalRequirement;
pub(crate) use codex_tool_runtime_api::PermissionRequestPayload;
pub(crate) use codex_tool_runtime_api::SandboxAttempt;
#[cfg(test)]
pub(crate) use codex_tool_runtime_api::SandboxOverride;
#[cfg(test)]
pub(crate) use codex_tool_runtime_api::Sandboxable;
pub(crate) type ToolCtx =
    codex_tool_runtime_api::ToolCtx<std::sync::Arc<Session>, std::sync::Arc<TurnContext>>;
pub(crate) use codex_tool_runtime_api::ToolError;
#[cfg(test)]
pub(crate) use codex_tool_runtime_api::ToolRuntime;
pub(crate) use codex_tool_runtime_api::managed_network_for_sandbox_permissions;
#[cfg(test)]
pub(crate) use codex_tool_runtime_api::sandbox_override_for_first_attempt;
use futures::Future;
use serde::Serialize;

pub(crate) fn permission_request_hook_payload(
    payload: PermissionRequestPayload,
) -> codex_hooks::PermissionRequestHookPayload {
    codex_hooks::PermissionRequestHookPayload {
        tool_name: payload.tool_name.name().to_string(),
        matcher_aliases: payload.tool_name.matcher_aliases().to_vec(),
        tool_input: payload.tool_input,
    }
}

/// Takes a vector of approval keys and returns a ReviewDecision.
/// There will be one key in most cases, but apply_patch can modify multiple files at once.
///
/// - If all keys are already approved for session, we skip prompting.
/// - If the user approves for session, we store the decision for each key individually
///   so future requests touching any subset can also skip prompting.
pub(crate) async fn with_cached_approval<K, F, Fut>(
    services: &SessionServices,
    // Name of the tool, used for metrics collection.
    tool_name: &str,
    keys: Vec<K>,
    fetch: F,
) -> ReviewDecision
where
    K: Serialize,
    F: FnOnce() -> Fut,
    Fut: Future<Output = ReviewDecision>,
{
    // To be defensive here, don't bother with checking the cache if keys are empty.
    if keys.is_empty() {
        return fetch().await;
    }

    let already_approved = {
        let store = services.tool_approvals.lock().await;
        keys.iter()
            .all(|key| matches!(store.get(key), Some(ReviewDecision::ApprovedForSession)))
    };

    if already_approved {
        return ReviewDecision::ApprovedForSession;
    }

    let decision = fetch().await;

    services.session_telemetry.counter(
        "codex.approval.requested",
        /*inc*/ 1,
        &[
            ("tool", tool_name),
            ("approved", decision.to_opaque_string()),
        ],
    );

    if matches!(decision, ReviewDecision::ApprovedForSession) {
        let mut store = services.tool_approvals.lock().await;
        for key in keys {
            store.put(key, ReviewDecision::ApprovedForSession);
        }
    }

    decision
}

pub(crate) trait SandboxAttemptExt {
    fn env_for(
        &self,
        command: SandboxCommand,
        options: ExecOptions,
        network: Option<SharedNetworkProxyRuntime>,
    ) -> Result<crate::sandboxing::ExecRequest, SandboxTransformError>;
}

impl SandboxAttemptExt for SandboxAttempt<'_> {
    fn env_for(
        &self,
        command: SandboxCommand,
        options: ExecOptions,
        network: Option<SharedNetworkProxyRuntime>,
    ) -> Result<crate::sandboxing::ExecRequest, SandboxTransformError> {
        let network_snapshot = network.as_ref().map(|network| network.runtime_snapshot());
        self.sandbox_runtime
            .transform(SandboxTransformRequest {
                command,
                permissions: self.permissions,
                sandbox: self.sandbox,
                enforce_managed_network: self.enforce_managed_network,
                network: network_snapshot.as_ref(),
                sandbox_policy_cwd: self.sandbox_cwd,
                codex_linux_sandbox_exe: self
                    .codex_linux_sandbox_exe
                    .map(std::path::PathBuf::as_path),
                use_legacy_landlock: self.use_legacy_landlock,
                windows_sandbox_level: self.windows_sandbox_level,
                windows_sandbox_private_desktop: self.windows_sandbox_private_desktop,
            })
            .map(|request| {
                let windows_sandbox_policy_cwd =
                    codex_utils_absolute_path::AbsolutePathBuf::try_from(
                        self.sandbox_cwd.to_path_buf(),
                    )
                    .unwrap_or_else(|_| request.cwd.clone());
                crate::sandboxing::ExecRequest::from_sandbox_exec_request(
                    request,
                    options,
                    windows_sandbox_policy_cwd,
                    network,
                )
            })
    }
}

#[cfg(test)]
#[path = "tool_runtime_support_tests.rs"]
mod tests;
