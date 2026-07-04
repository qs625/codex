use codex_config_types::AppToolApproval;
use protocol::config_types::ApprovalsReviewer;
use protocol::models::PermissionProfile;
use protocol::protocol::AskForApproval;

/// Request context used to decide whether an MCP permission prompt can be
/// resolved without surfacing a user prompt.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct McpPermissionPromptAutoApproveContext {
    pub approvals_reviewer: Option<ApprovalsReviewer>,
    pub tool_approval_mode: Option<AppToolApproval>,
}

/// Returns true when an MCP permission prompt should resolve as approved
/// instead of being shown to the user.
pub fn mcp_permission_prompt_is_auto_approved(
    approval_policy: AskForApproval,
    permission_profile: &PermissionProfile,
    context: McpPermissionPromptAutoApproveContext,
) -> bool {
    if context.tool_approval_mode == Some(AppToolApproval::Approve) {
        return true;
    }

    if approval_policy != AskForApproval::Never {
        return false;
    }

    match permission_profile {
        PermissionProfile::Disabled | PermissionProfile::External { .. } => true,
        PermissionProfile::Managed { file_system, .. } => {
            file_system.to_sandbox_policy().has_full_disk_write_access()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_config_types::AppToolApproval;
    use protocol::config_types::ApprovalsReviewer;
    use protocol::models::ManagedFileSystemPermissions;
    use protocol::models::PermissionProfile;
    use protocol::permissions::NetworkSandboxPolicy;
    use protocol::protocol::AskForApproval;
    use protocol::protocol::GranularApprovalConfig;

    #[test]
    fn auto_approval_honors_unrestricted_managed_profiles() {
        assert!(mcp_permission_prompt_is_auto_approved(
            AskForApproval::Never,
            &PermissionProfile::Managed {
                file_system: ManagedFileSystemPermissions::Unrestricted,
                network: NetworkSandboxPolicy::Enabled,
            },
            McpPermissionPromptAutoApproveContext::default(),
        ));
        assert!(mcp_permission_prompt_is_auto_approved(
            AskForApproval::Never,
            &PermissionProfile::Managed {
                file_system: ManagedFileSystemPermissions::Unrestricted,
                network: NetworkSandboxPolicy::Restricted,
            },
            McpPermissionPromptAutoApproveContext::default(),
        ));
        assert!(!mcp_permission_prompt_is_auto_approved(
            AskForApproval::Never,
            &PermissionProfile::read_only(),
            McpPermissionPromptAutoApproveContext::default(),
        ));
        assert!(!mcp_permission_prompt_is_auto_approved(
            AskForApproval::OnRequest,
            &PermissionProfile::Managed {
                file_system: ManagedFileSystemPermissions::Unrestricted,
                network: NetworkSandboxPolicy::Enabled,
            },
            McpPermissionPromptAutoApproveContext::default(),
        ));
    }

    #[test]
    fn auto_approval_honors_approved_tools_in_all_permission_modes() {
        for approval_policy in [
            AskForApproval::UnlessTrusted,
            AskForApproval::OnFailure,
            AskForApproval::OnRequest,
            AskForApproval::Granular(GranularApprovalConfig {
                sandbox_approval: true,
                rules: true,
                skill_approval: true,
                request_permissions: true,
                mcp_elicitations: true,
            }),
            AskForApproval::Never,
        ] {
            assert!(mcp_permission_prompt_is_auto_approved(
                approval_policy,
                &PermissionProfile::read_only(),
                McpPermissionPromptAutoApproveContext {
                    approvals_reviewer: Some(ApprovalsReviewer::User),
                    tool_approval_mode: Some(AppToolApproval::Approve),
                },
            ));
        }

        assert!(!mcp_permission_prompt_is_auto_approved(
            AskForApproval::OnRequest,
            &PermissionProfile::read_only(),
            McpPermissionPromptAutoApproveContext {
                approvals_reviewer: Some(ApprovalsReviewer::AutoReview),
                tool_approval_mode: Some(AppToolApproval::Auto),
            },
        ));
    }

    #[test]
    fn auto_approval_rejects_auto_mode_in_default_permission_mode() {
        assert!(!mcp_permission_prompt_is_auto_approved(
            AskForApproval::OnRequest,
            &PermissionProfile::read_only(),
            McpPermissionPromptAutoApproveContext {
                approvals_reviewer: Some(ApprovalsReviewer::User),
                tool_approval_mode: Some(AppToolApproval::Auto),
            },
        ));
    }
}
