use crate::Approvable;
use crate::ApprovalCtx;
use crate::ExecApprovalRequirement;
use crate::HookToolName;
use crate::PermissionRequestPayload;
use crate::SandboxAttempt;
use crate::Sandboxable;
use crate::ToolCtx;
use crate::ToolError;
use crate::ToolRuntime;
use codex_apply_patch::AppliedPatchDelta;
use codex_apply_patch::ApplyPatchAction;
use codex_apply_patch::ApplyPatchFileChange;
use codex_command_runtime::is_likely_sandbox_denied;
use codex_file_system::FileSystemSandboxContext;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::error::CodexErr;
use codex_protocol::error::SandboxErr;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_protocol::exec_output::StreamOutput;
use codex_protocol::models::PermissionProfile;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::FileChange;
use codex_protocol::protocol::ReviewDecision;
use codex_sandboxing_api::SandboxType;
use codex_sandboxing_api::SandboxablePreference;
use codex_sandboxing_api::policy_transforms::effective_permission_profile;
pub use codex_tool_runtime_api::ApplyPatchApprovalKey;
pub use codex_tool_runtime_api::ApplyPatchApprovalRequest;
pub use codex_tool_runtime_api::ApplyPatchEnvironment;
pub use codex_tool_runtime_api::ApplyPatchRequest;
pub use codex_tool_runtime_api::ApplyPatchRuntimeHost;
use codex_utils_absolute_path::AbsolutePathBuf;
use futures::future::BoxFuture;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use crate::SafetyCheck;
use crate::assess_patch_safety;

#[derive(Default)]
pub struct ApplyPatchRuntime<Host> {
    host: Host,
    committed_delta: AppliedPatchDelta,
}

pub struct ApplyPatchRuntimeOutput {
    pub exec_output: ExecToolCallOutput,
    pub delta: AppliedPatchDelta,
}

#[derive(Debug)]
pub struct ApplyPatchRuntimeInvocation {
    pub action: ApplyPatchAction,
    pub auto_approved: bool,
    pub exec_approval_requirement: ExecApprovalRequirement,
}

#[derive(Debug)]
pub enum ApplyPatchPlan {
    DelegateToRuntime(ApplyPatchRuntimeInvocation),
    Reject { reason: String },
}

impl<Host> ApplyPatchRuntime<Host> {
    pub fn new(host: Host) -> Self {
        Self {
            host,
            committed_delta: AppliedPatchDelta::default(),
        }
    }

    pub fn committed_delta(&self) -> &AppliedPatchDelta {
        &self.committed_delta
    }

    pub fn file_system_sandbox_context_for_attempt(
        req: &ApplyPatchRequest,
        attempt: &SandboxAttempt<'_>,
    ) -> Option<FileSystemSandboxContext> {
        if attempt.sandbox == SandboxType::None {
            return None;
        }

        let permissions =
            effective_permission_profile(attempt.permissions, req.additional_permissions.as_ref());
        Some(FileSystemSandboxContext {
            permissions,
            cwd: Some(attempt.sandbox_cwd.clone()),
            windows_sandbox_level: attempt.windows_sandbox_level,
            windows_sandbox_private_desktop: attempt.windows_sandbox_private_desktop,
            use_legacy_landlock: attempt.use_legacy_landlock,
        })
    }
}

impl<Host> Sandboxable for ApplyPatchRuntime<Host>
where
    Host: Send + Sync,
{
    fn sandbox_preference(&self) -> SandboxablePreference {
        SandboxablePreference::Auto
    }

    fn escalate_on_failure(&self) -> bool {
        true
    }
}

impl<Host> Approvable<ApplyPatchRequest> for ApplyPatchRuntime<Host>
where
    Host: ApplyPatchRuntimeHost,
{
    type Session = Host::Session;
    type Turn = Host::Turn;
    type ApprovalKey = ApplyPatchApprovalKey;

    fn approval_keys(&self, req: &ApplyPatchRequest) -> Vec<Self::ApprovalKey> {
        with_apply_patch_approval_keys(req.environment.environment_id(), &req.file_paths)
    }

    fn start_approval_async<'a>(
        &'a mut self,
        req: &'a ApplyPatchRequest,
        ctx: ApprovalCtx<'a, Self::Session, Self::Turn>,
    ) -> BoxFuture<'a, ReviewDecision> {
        let keys = self.approval_keys(req);
        let approval_request = ApplyPatchApprovalRequest::from_request(req);
        self.host
            .start_apply_patch_approval_async(req, ctx, keys, approval_request)
    }

    fn wants_no_sandbox_approval(&self, policy: AskForApproval) -> bool {
        match policy {
            AskForApproval::Never => false,
            AskForApproval::Granular(granular_config) => granular_config.allows_sandbox_approval(),
            AskForApproval::OnFailure => true,
            AskForApproval::OnRequest => true,
            AskForApproval::UnlessTrusted => true,
        }
    }

    fn exec_approval_requirement(
        &self,
        req: &ApplyPatchRequest,
    ) -> Option<ExecApprovalRequirement> {
        Some(req.exec_approval_requirement.clone())
    }

    fn permission_request_payload(
        &self,
        req: &ApplyPatchRequest,
    ) -> Option<PermissionRequestPayload> {
        Some(PermissionRequestPayload {
            tool_name: HookToolName::apply_patch(),
            tool_input: serde_json::json!({ "command": req.action.patch }),
        })
    }
}

impl<Host> ToolRuntime<ApplyPatchRequest, ApplyPatchRuntimeOutput> for ApplyPatchRuntime<Host>
where
    Host: ApplyPatchRuntimeHost,
{
    type NetworkApprovalTrigger = Host::NetworkApprovalTrigger;

    fn sandbox_cwd<'a>(&self, req: &'a ApplyPatchRequest) -> Option<&'a AbsolutePathBuf> {
        Some(&req.action.cwd)
    }

    async fn run(
        &mut self,
        req: &ApplyPatchRequest,
        attempt: &SandboxAttempt<'_>,
        _ctx: &ToolCtx<Self::Session, Self::Turn>,
    ) -> Result<ApplyPatchRuntimeOutput, ToolError> {
        let started_at = Instant::now();
        let fs = req.environment.filesystem();
        let sandbox = Self::file_system_sandbox_context_for_attempt(req, attempt);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = codex_apply_patch::apply_patch(
            &req.action.patch,
            &req.action.cwd,
            &mut stdout,
            &mut stderr,
            fs.as_ref(),
            sandbox.as_ref(),
        )
        .await;
        let stdout = String::from_utf8_lossy(&stdout).into_owned();
        let stderr = String::from_utf8_lossy(&stderr).into_owned();
        let failed = result.is_err();
        let exit_code = if failed { 1 } else { 0 };
        let delta = match result {
            Ok(delta) => delta,
            Err(failure) => failure.into_parts().1,
        };
        self.committed_delta.append(delta);
        let output = ExecToolCallOutput {
            exit_code,
            stdout: StreamOutput::new(stdout.clone()),
            stderr: StreamOutput::new(stderr.clone()),
            aggregated_output: StreamOutput::new(format!("{stdout}{stderr}")),
            duration: started_at.elapsed(),
            timed_out: false,
        };
        if failed && is_likely_sandbox_denied(attempt.sandbox, &output) {
            return Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied {
                output: Box::new(output),
                network_policy_decision: None,
            })));
        }
        Ok(ApplyPatchRuntimeOutput {
            exec_output: output,
            delta: self.committed_delta.clone(),
        })
    }
}

pub fn with_apply_patch_approval_keys(
    environment_id: &str,
    file_paths: &[AbsolutePathBuf],
) -> Vec<ApplyPatchApprovalKey> {
    file_paths
        .iter()
        .cloned()
        .map(|path| ApplyPatchApprovalKey {
            environment_id: environment_id.to_string(),
            path,
        })
        .collect()
}

pub fn plan_apply_patch(
    action: ApplyPatchAction,
    policy: AskForApproval,
    permission_profile: &PermissionProfile,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    cwd: &AbsolutePathBuf,
    windows_sandbox_level: WindowsSandboxLevel,
) -> ApplyPatchPlan {
    match assess_patch_safety(
        &action,
        policy,
        permission_profile,
        file_system_sandbox_policy,
        cwd,
        windows_sandbox_level,
    ) {
        SafetyCheck::AutoApprove {
            user_explicitly_approved,
            ..
        } => ApplyPatchPlan::DelegateToRuntime(ApplyPatchRuntimeInvocation {
            action,
            auto_approved: !user_explicitly_approved,
            exec_approval_requirement: ExecApprovalRequirement::Skip {
                bypass_sandbox: false,
                proposed_execpolicy_amendment: None,
            },
        }),
        SafetyCheck::AskUser => ApplyPatchPlan::DelegateToRuntime(ApplyPatchRuntimeInvocation {
            action,
            auto_approved: false,
            exec_approval_requirement: ExecApprovalRequirement::NeedsApproval {
                reason: None,
                proposed_execpolicy_amendment: None,
            },
        }),
        SafetyCheck::Reject { reason } => ApplyPatchPlan::Reject { reason },
    }
}

pub fn convert_apply_patch_to_protocol(action: &ApplyPatchAction) -> HashMap<PathBuf, FileChange> {
    let mut result = HashMap::with_capacity(action.changes().len());
    for (path, change) in action.changes() {
        let protocol_change = match change {
            ApplyPatchFileChange::Add { content, .. } => FileChange::Add {
                content: content.clone(),
            },
            ApplyPatchFileChange::Delete { content } => FileChange::Delete {
                content: content.clone(),
            },
            ApplyPatchFileChange::Update {
                unified_diff,
                move_path,
                new_content: _new_content,
            } => FileChange::Update {
                unified_diff: unified_diff.clone(),
                move_path: move_path.clone(),
            },
        };
        result.insert(path.to_path_buf(), protocol_change);
    }
    result
}

#[cfg(test)]
mod conversion_tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn convert_apply_patch_maps_add_variant() {
        let tmp = tempfile::tempdir().expect("tmp");
        let p = AbsolutePathBuf::try_from(tmp.path().join("a.txt")).expect("absolute temp path");
        let action = ApplyPatchAction::new_add_for_test(&p, "hello".to_string());

        let got = convert_apply_patch_to_protocol(&action);

        assert_eq!(
            got.get(p.as_path()),
            Some(&FileChange::Add {
                content: "hello".to_string()
            })
        );
    }
}
