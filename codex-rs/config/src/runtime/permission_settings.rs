use std::path::Path;

use codex_config_types::Constrained;
use codex_config_types::ConstraintResult;
use codex_config_types::WindowsSandboxModeToml;
use codex_protocol::config_types::ShellEnvironmentPolicy;
use codex_protocol::models::ActivePermissionProfile;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::SandboxEnforcement;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::SandboxPolicy;
use codex_sandboxing_api::compatibility_sandbox_policy_for_permission_profile;
use codex_utils_absolute_path::AbsolutePathBuf;

use super::NetworkProxySpec;
use super::PermissionProfileState;

/// Application permission configuration loaded from disk and merged with overrides.
#[derive(Debug, Clone, PartialEq)]
pub struct Permissions {
    /// Approval policy for executing commands.
    pub approval_policy: Constrained<AskForApproval>,
    /// Constrained permission profile plus its selected profile identity, if
    /// the profile came from a built-in or named config profile.
    pub(super) permission_profile_state: PermissionProfileState,
    /// Thread-scoped runtime workspace roots. Symbolic `:workspace_roots`
    /// entries in the permission profile are materialized against these roots.
    pub(super) workspace_roots: Vec<AbsolutePathBuf>,
    /// Effective network configuration applied to all spawned processes.
    pub network: Option<NetworkProxySpec>,
    /// Whether the model may request a login shell for shell-based tools.
    /// Default to `true`
    ///
    /// If `true`, the model may request a login shell (`login = true`), and
    /// omitting `login` defaults to using a login shell.
    /// If `false`, the model can never use a login shell: `login = true`
    /// requests are rejected, and omitting `login` defaults to a non-login
    /// shell.
    pub allow_login_shell: bool,
    /// Policy used to build process environments for shell/unified exec.
    pub shell_environment_policy: ShellEnvironmentPolicy,
    /// Effective Windows sandbox mode derived from `[windows].sandbox` or
    /// legacy feature keys.
    pub windows_sandbox_mode: Option<WindowsSandboxModeToml>,
    /// Whether the final Windows sandboxed child should run on a private desktop.
    pub windows_sandbox_private_desktop: bool,
}

impl Permissions {
    /// Build permissions from the constrained values required for a minimal
    /// in-process configuration.
    pub fn from_approval_and_profile(
        approval_policy: Constrained<AskForApproval>,
        permission_profile: Constrained<PermissionProfile>,
    ) -> ConstraintResult<Self> {
        Ok(Self {
            approval_policy,
            permission_profile_state: PermissionProfileState::from_constrained_legacy(
                permission_profile,
            )?,
            workspace_roots: Vec::new(),
            network: None,
            allow_login_shell: true,
            shell_environment_policy: ShellEnvironmentPolicy::default(),
            windows_sandbox_mode: None,
            windows_sandbox_private_desktop: true,
        })
    }

    pub fn permission_profile_state(&self) -> &PermissionProfileState {
        &self.permission_profile_state
    }

    pub fn set_permission_profile_state(
        &mut self,
        permission_profile_state: PermissionProfileState,
    ) {
        self.permission_profile_state = permission_profile_state;
    }

    /// Apply a permission profile snapshot emitted by core session state.
    ///
    /// This is a trusted-state bridge for consumers of `SessionConfigured`.
    /// Config loading and app-server selection should resolve named profiles
    /// through config instead of constructing this pair directly.
    pub fn set_permission_profile_from_session_snapshot(
        &mut self,
        permission_profile: PermissionProfile,
        active_permission_profile: Option<ActivePermissionProfile>,
    ) -> ConstraintResult<()> {
        self.set_permission_profile_from_session_snapshot_with_profile_workspace_roots(
            permission_profile,
            active_permission_profile,
            Vec::new(),
        )
    }

    pub fn set_permission_profile_from_session_snapshot_with_profile_workspace_roots(
        &mut self,
        permission_profile: PermissionProfile,
        active_permission_profile: Option<ActivePermissionProfile>,
        profile_workspace_roots: Vec<AbsolutePathBuf>,
    ) -> ConstraintResult<()> {
        self.permission_profile_state.set_active_permission_profile(
            permission_profile,
            active_permission_profile,
            profile_workspace_roots,
        )
    }

    /// Replace the current permission constraints with a trusted session
    /// snapshot. This is only for clients that must mirror core session state
    /// after their local config constraints reject the snapshot.
    pub fn replace_permission_profile_from_session_snapshot(
        &mut self,
        permission_profile: Constrained<PermissionProfile>,
        active_permission_profile: Option<ActivePermissionProfile>,
    ) -> ConstraintResult<()> {
        self.replace_permission_profile_from_session_snapshot_with_profile_workspace_roots(
            permission_profile,
            active_permission_profile,
            Vec::new(),
        )
    }

    pub fn replace_permission_profile_from_session_snapshot_with_profile_workspace_roots(
        &mut self,
        permission_profile: Constrained<PermissionProfile>,
        active_permission_profile: Option<ActivePermissionProfile>,
        profile_workspace_roots: Vec<AbsolutePathBuf>,
    ) -> ConstraintResult<()> {
        self.permission_profile_state = PermissionProfileState::from_constrained_active_profile(
            permission_profile,
            active_permission_profile,
            profile_workspace_roots,
        )?;
        Ok(())
    }

    /// Borrow the canonical profile before runtime workspace-root
    /// materialization has been applied.
    pub fn permission_profile(&self) -> &PermissionProfile {
        self.permission_profile_state.permission_profile()
    }

    pub fn can_set_permission_profile(
        &self,
        permission_profile: &PermissionProfile,
    ) -> ConstraintResult<()> {
        self.permission_profile_state
            .can_set_legacy_permission_profile(permission_profile)
    }

    pub fn set_workspace_roots(&mut self, workspace_roots: Vec<AbsolutePathBuf>) {
        self.workspace_roots = workspace_roots;
    }

    pub fn workspace_roots(&self) -> &[AbsolutePathBuf] {
        &self.workspace_roots
    }

    /// Workspace roots that came from user-visible configuration or runtime
    /// selection. Internal Codex-only writable roots are intentionally excluded.
    pub fn user_visible_workspace_roots(&self) -> &[AbsolutePathBuf] {
        &self.workspace_roots
    }

    pub fn profile_workspace_roots(&self) -> &[AbsolutePathBuf] {
        self.permission_profile_state.profile_workspace_roots()
    }

    fn materialized_permission_profile(&self) -> PermissionProfile {
        self.permission_profile()
            .clone()
            .materialize_project_roots_with_workspace_roots(&self.workspace_roots)
    }

    /// Effective runtime permissions after config requirements and runtime
    /// workspace-root materialization have been applied.
    pub fn effective_permission_profile(&self) -> PermissionProfile {
        self.materialized_permission_profile()
    }

    /// Named profile selected by config, if the current profile has one.
    pub fn active_permission_profile(&self) -> Option<ActivePermissionProfile> {
        self.permission_profile_state.active_permission_profile()
    }

    /// Effective filesystem sandbox policy derived from the canonical profile.
    pub fn file_system_sandbox_policy(&self) -> FileSystemSandboxPolicy {
        self.materialized_permission_profile()
            .file_system_sandbox_policy()
    }

    /// Effective network sandbox policy derived from the canonical profile.
    pub fn network_sandbox_policy(&self) -> NetworkSandboxPolicy {
        self.permission_profile().network_sandbox_policy()
    }

    /// Legacy compatibility projection derived from the canonical profile.
    pub fn legacy_sandbox_policy(&self, cwd: &Path) -> SandboxPolicy {
        let permission_profile = self.materialized_permission_profile();
        let file_system_sandbox_policy = permission_profile.file_system_sandbox_policy();
        compatibility_sandbox_policy_for_permission_profile(
            &permission_profile,
            &file_system_sandbox_policy,
            permission_profile.network_sandbox_policy(),
            cwd,
        )
    }

    /// Check whether a legacy sandbox policy can be applied to this permission
    /// set after projecting it into the canonical permission profile.
    pub fn can_set_legacy_sandbox_policy(
        &self,
        sandbox_policy: &SandboxPolicy,
        cwd: &Path,
    ) -> ConstraintResult<()> {
        let file_system_sandbox_policy =
            FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd(sandbox_policy, cwd);
        let network_sandbox_policy = NetworkSandboxPolicy::from(sandbox_policy);
        let permission_profile = PermissionProfile::from_runtime_permissions_with_enforcement(
            SandboxEnforcement::from_legacy_sandbox_policy(sandbox_policy),
            &file_system_sandbox_policy,
            network_sandbox_policy,
        );
        self.permission_profile_state
            .can_set_legacy_permission_profile(&permission_profile)
    }

    /// Set permissions from a legacy sandbox policy and keep every permission
    /// projection in sync.
    pub fn set_legacy_sandbox_policy(
        &mut self,
        sandbox_policy: SandboxPolicy,
        cwd: &Path,
    ) -> ConstraintResult<()> {
        self.can_set_legacy_sandbox_policy(&sandbox_policy, cwd)?;
        let file_system_sandbox_policy =
            FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd(&sandbox_policy, cwd);
        let network_sandbox_policy = NetworkSandboxPolicy::from(&sandbox_policy);
        let permission_profile = PermissionProfile::from_runtime_permissions_with_enforcement(
            SandboxEnforcement::from_legacy_sandbox_policy(&sandbox_policy),
            &file_system_sandbox_policy,
            network_sandbox_policy,
        );
        self.workspace_roots = match &sandbox_policy {
            SandboxPolicy::WorkspaceWrite { writable_roots, .. } => {
                let mut workspace_roots = vec![
                    AbsolutePathBuf::from_absolute_path(cwd)
                        .unwrap_or_else(|_| AbsolutePathBuf::resolve_path_against_base(cwd, "/")),
                ];
                for root in writable_roots {
                    if !workspace_roots.iter().any(|existing| existing == root) {
                        workspace_roots.push(root.clone());
                    }
                }
                workspace_roots
            }
            SandboxPolicy::DangerFullAccess
            | SandboxPolicy::ExternalSandbox { .. }
            | SandboxPolicy::ReadOnly { .. } => vec![
                AbsolutePathBuf::from_absolute_path(cwd)
                    .unwrap_or_else(|_| AbsolutePathBuf::resolve_path_against_base(cwd, "/")),
            ],
        };

        self.permission_profile_state
            .set_legacy_permission_profile(permission_profile)?;
        Ok(())
    }

    /// Set permissions from the canonical profile.
    pub fn set_permission_profile(
        &mut self,
        permission_profile: PermissionProfile,
    ) -> ConstraintResult<()> {
        self.permission_profile_state
            .set_legacy_permission_profile(permission_profile)
    }
}

// A profile override only inherits the selected profile's proxy/allowlist config
// when Codex is still responsible for the network policy. `Disabled` means no
// outer sandbox, so starting the managed proxy would narrow the override.
pub(super) fn profile_allows_configured_network_proxy(
    permission_profile: &PermissionProfile,
) -> bool {
    match permission_profile {
        PermissionProfile::Managed { network, .. } | PermissionProfile::External { network } => {
            network.is_enabled()
        }
        PermissionProfile::Disabled => false,
    }
}
