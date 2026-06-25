use std::sync::Arc;

use crate::function_tool::FunctionCallError;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::context::SharedTurnDiffTracker;
use crate::tools::events::CoreToolEventHost;
use crate::tools::orchestrator::CoreToolOrchestratorHost;
use crate::tools::runtimes::CoreToolRuntimeHost;
use codex_file_system::FileSystemSandboxContext;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::PermissionProfile;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_tool_runtime_api::ApplyPatchDiffContext;
use codex_tool_runtime_api::ApplyPatchHandlerHost;
use codex_tool_runtime_api::ResolvedApplyPatchEnvironment;
use codex_tool_runtime_api::ToolPermissionGrants;
use codex_tool_runtime_api::ToolSandboxContext;
use codex_utils_absolute_path::AbsolutePathBuf;

#[cfg(test)]
pub(crate) use codex_tool_runtime::ApplyPatchToolOutput;

/// Core adapter for the host-neutral apply-patch handler owned by
/// `codex-tool-runtime`.
#[derive(Clone, Copy, Default)]
pub struct CoreApplyPatchHandlerHost;

impl ApplyPatchDiffContext for TurnContext {
    fn apply_patch_streaming_events_enabled(&self) -> bool {
        self.is_apply_patch_streaming_events_enabled()
    }
}

impl ApplyPatchHandlerHost for CoreApplyPatchHandlerHost {
    type Session = Arc<Session>;
    type Turn = Arc<TurnContext>;
    type Tracker = SharedTurnDiffTracker;
    type DiffContext = TurnContext;
    type RuntimeHost = CoreToolRuntimeHost;
    type OrchestratorHost = CoreToolOrchestratorHost;
    type EventHost<'a> = CoreToolEventHost<'a>;

    fn runtime_host(&self) -> Self::RuntimeHost {
        CoreToolRuntimeHost
    }

    fn orchestrator_host(&self) -> Self::OrchestratorHost {
        CoreToolOrchestratorHost
    }

    fn sandbox_runtime(&self, session: &Self::Session) -> SharedSandboxRuntime {
        session.sandbox_runtime()
    }

    fn tool_sandbox_context(&self, turn: &Self::Turn) -> ToolSandboxContext {
        turn.tool_sandbox_context()
    }

    fn approval_policy(&self, turn: &Self::Turn) -> AskForApproval {
        turn.approval_policy()
    }

    fn permission_profile(&self, turn: &Self::Turn) -> PermissionProfile {
        turn.permission_profile()
    }

    fn file_system_sandbox_policy(&self, turn: &Self::Turn) -> FileSystemSandboxPolicy {
        turn.file_system_sandbox_policy()
    }

    fn windows_sandbox_level(&self, turn: &Self::Turn) -> WindowsSandboxLevel {
        turn.windows_sandbox_level()
    }

    fn file_system_sandbox_context(
        &self,
        turn: &Self::Turn,
        additional_permissions: Option<AdditionalPermissionProfile>,
        cwd: &AbsolutePathBuf,
    ) -> FileSystemSandboxContext {
        turn.file_system_sandbox_context(additional_permissions, cwd)
    }

    fn resolve_environment(
        &self,
        turn: &Self::Turn,
        environment_id: Option<&str>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, FunctionCallError> {
        turn.resolve_apply_patch_environment(environment_id)
    }

    async fn permission_grants(&self, session: &Self::Session) -> ToolPermissionGrants {
        ToolPermissionGrants {
            session: session.granted_session_permissions().await,
            turn: session.granted_turn_permissions().await,
        }
    }

    fn event_host<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        tracker: Option<&'a Self::Tracker>,
    ) -> Self::EventHost<'a> {
        CoreToolEventHost::new(session.as_ref(), turn.as_ref(), tracker)
    }
}

#[cfg(test)]
#[path = "apply_patch_tool_host_tests.rs"]
mod tests;
