use codex_apply_patch::ApplyPatchAction;
use codex_apply_patch::ApplyPatchFileChange;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_permissions_runtime::ExecApprovalRequirement;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::error::CodexErr;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::FileChange;
use codex_sandboxing_api::get_platform_sandbox;
use codex_sandboxing_api::ApplyPatchEnvironment;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::collections::HashMap;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug)]
pub(crate) enum ToolError {
    Rejected(String),
    Codex(CodexErr),
}

pub(crate) struct ApplyPatchRequest {
    pub(crate) environment: Arc<dyn ApplyPatchEnvironment>,
    pub(crate) action: ApplyPatchAction,
    pub(crate) file_paths: Vec<AbsolutePathBuf>,
    pub(crate) changes: HashMap<PathBuf, FileChange>,
    pub(crate) exec_approval_requirement: ExecApprovalRequirement,
    pub(crate) additional_permissions: Option<AdditionalPermissionProfile>,
    pub(crate) permissions_preapproved: bool,
}

pub(crate) struct ApplyPatchRuntimeInvocation {
    pub(crate) action: ApplyPatchAction,
    pub(crate) auto_approved: bool,
    pub(crate) exec_approval_requirement: ExecApprovalRequirement,
}

pub(crate) enum ApplyPatchPlan {
    DelegateToRuntime(ApplyPatchRuntimeInvocation),
    Reject { reason: String },
}

enum SafetyCheck {
    AutoApprove { user_explicitly_approved: bool },
    AskUser,
    Reject { reason: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SandboxOverride {
    NoOverride,
    BypassSandboxFirstAttempt,
}

pub(crate) fn sandbox_override_for_first_attempt(
    sandbox_permissions: SandboxPermissions,
    exec_approval_requirement: &ExecApprovalRequirement,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
) -> SandboxOverride {
    if matches!(
        exec_approval_requirement,
        ExecApprovalRequirement::Skip {
            bypass_sandbox: true,
            ..
        }
    ) {
        return SandboxOverride::BypassSandboxFirstAttempt;
    }
    if file_system_sandbox_policy.has_denied_read_restrictions() {
        return SandboxOverride::NoOverride;
    }
    if sandbox_permissions.requires_escalated_permissions() {
        SandboxOverride::BypassSandboxFirstAttempt
    } else {
        SandboxOverride::NoOverride
    }
}

pub(crate) fn should_bypass_approval(policy: AskForApproval, already_approved: bool) -> bool {
    already_approved || matches!(policy, AskForApproval::Never)
}

pub(crate) fn wants_no_sandbox_approval(policy: AskForApproval) -> bool {
    match policy {
        AskForApproval::OnFailure => true,
        AskForApproval::UnlessTrusted => true,
        AskForApproval::Never => false,
        AskForApproval::OnRequest => false,
        AskForApproval::Granular(granular_config) => granular_config.sandbox_approval,
    }
}

pub(crate) fn managed_network_for_sandbox_permissions(
    network: Option<&SharedNetworkProxyRuntime>,
    sandbox_permissions: SandboxPermissions,
) -> Option<SharedNetworkProxyRuntime> {
    if sandbox_permissions.requires_escalated_permissions() {
        None
    } else {
        network.cloned()
    }
}

pub(crate) fn plan_apply_patch(
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

pub(crate) fn convert_apply_patch_to_protocol(
    action: &ApplyPatchAction,
) -> HashMap<PathBuf, FileChange> {
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
                new_content: _,
            } => FileChange::Update {
                unified_diff: unified_diff.clone(),
                move_path: move_path.clone(),
            },
        };
        result.insert(path.to_path_buf(), protocol_change);
    }
    result
}

fn assess_patch_safety(
    action: &ApplyPatchAction,
    policy: AskForApproval,
    permission_profile: &PermissionProfile,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    cwd: &AbsolutePathBuf,
    windows_sandbox_level: WindowsSandboxLevel,
) -> SafetyCheck {
    if action.is_empty() {
        return SafetyCheck::Reject {
            reason: "empty patch".to_string(),
        };
    }

    match policy {
        AskForApproval::OnFailure
        | AskForApproval::Never
        | AskForApproval::OnRequest
        | AskForApproval::Granular(_) => {}
        AskForApproval::UnlessTrusted => {
            return SafetyCheck::AskUser;
        }
    }

    let rejects_sandbox_approval = matches!(policy, AskForApproval::Never)
        || matches!(
            policy,
            AskForApproval::Granular(granular_config) if !granular_config.sandbox_approval
        );

    if is_write_patch_constrained_to_writable_paths(action, file_system_sandbox_policy, cwd)
        || matches!(policy, AskForApproval::OnFailure)
    {
        if matches!(
            permission_profile,
            PermissionProfile::Disabled | PermissionProfile::External { .. }
        ) {
            SafetyCheck::AutoApprove {
                user_explicitly_approved: false,
            }
        } else {
            match get_platform_sandbox(windows_sandbox_level != WindowsSandboxLevel::Disabled) {
                Some(_sandbox_type) => SafetyCheck::AutoApprove {
                    user_explicitly_approved: false,
                },
                None => {
                    if rejects_sandbox_approval {
                        SafetyCheck::Reject {
                            reason: patch_rejection_reason(
                                permission_profile,
                                file_system_sandbox_policy,
                                cwd,
                            )
                            .to_string(),
                        }
                    } else {
                        SafetyCheck::AskUser
                    }
                }
            }
        }
    } else if rejects_sandbox_approval {
        SafetyCheck::Reject {
            reason: patch_rejection_reason(permission_profile, file_system_sandbox_policy, cwd)
                .to_string(),
        }
    } else {
        SafetyCheck::AskUser
    }
}

fn patch_rejection_reason(
    permission_profile: &PermissionProfile,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    cwd: &AbsolutePathBuf,
) -> &'static str {
    match permission_profile {
        PermissionProfile::Managed { .. }
            if !file_system_sandbox_policy.has_full_disk_write_access()
                && file_system_sandbox_policy
                    .get_writable_roots_with_cwd(cwd.as_path())
                    .is_empty() =>
        {
            "writing is blocked by read-only sandbox; rejected by user approval settings"
        }
        PermissionProfile::Managed { .. }
        | PermissionProfile::Disabled
        | PermissionProfile::External { .. } => {
            "writing outside of the project; rejected by user approval settings"
        }
    }
}

fn is_write_patch_constrained_to_writable_paths(
    action: &ApplyPatchAction,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    cwd: &AbsolutePathBuf,
) -> bool {
    fn normalize(path: &Path) -> PathBuf {
        let mut out = PathBuf::new();
        for comp in path.components() {
            match comp {
                Component::ParentDir => {
                    out.pop();
                }
                Component::CurDir => {}
                other => out.push(other.as_os_str()),
            }
        }
        out
    }

    let is_path_writable = |path: &Path| {
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path).to_path_buf()
        };
        file_system_sandbox_policy.can_write_path_with_cwd(&normalize(&abs), cwd)
    };

    for (path, change) in action.changes() {
        match change {
            ApplyPatchFileChange::Add { .. } | ApplyPatchFileChange::Delete { .. } => {
                if !is_path_writable(path) {
                    return false;
                }
            }
            ApplyPatchFileChange::Update { move_path, .. } => {
                if !is_path_writable(path) {
                    return false;
                }
                if let Some(dest) = move_path
                    && !is_path_writable(dest)
                {
                    return false;
                }
            }
        }
    }
    true
}
