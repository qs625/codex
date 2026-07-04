use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::account::PlanType as AccountPlanType;
use protocol::config_types::CollaborationMode;
use protocol::config_types::Personality;
use protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use protocol::config_types::ServiceTier;
use protocol::models::ActivePermissionProfile;
use protocol::models::PermissionProfile;
use protocol::models::SandboxEnforcement;
use protocol::permissions::FileSystemPath;
use protocol::permissions::FileSystemSandboxPolicy;
use protocol::permissions::FileSystemSpecialPath;
use protocol::permissions::NetworkSandboxPolicy;
use protocol::protocol::SandboxPolicy;

use crate::SessionSettingsUpdate;

pub struct SessionSettingsApplyCurrent<'a> {
    pub collaboration_mode: &'a CollaborationMode,
    pub service_tier: Option<String>,
    pub personality: Option<Personality>,
    pub cwd: &'a AbsolutePathBuf,
    pub workspace_roots: &'a [AbsolutePathBuf],
    pub permission_profile: &'a PermissionProfile,
    pub active_permission_profile: Option<ActivePermissionProfile>,
    pub sandbox_policy: &'a SandboxPolicy,
    pub file_system_sandbox_policy: &'a FileSystemSandboxPolicy,
    pub network_sandbox_policy: NetworkSandboxPolicy,
    pub app_server_client_name: Option<String>,
    pub app_server_client_version: Option<String>,
}

pub struct SessionSettingsApplyPlan {
    pub collaboration_mode: CollaborationMode,
    pub model_provider_update: Option<String>,
    pub model_reasoning_summary: Option<ReasoningSummaryConfig>,
    pub service_tier: Option<String>,
    pub personality: Option<Personality>,
    pub cwd: AbsolutePathBuf,
    pub workspace_roots: Vec<AbsolutePathBuf>,
    pub permission_profile_update: Option<SessionPermissionProfileUpdate>,
    pub app_server_client_name: Option<String>,
    pub app_server_client_version: Option<String>,
}

pub enum SessionPermissionProfileUpdate {
    ActiveProfile {
        permission_profile: PermissionProfile,
        active_permission_profile: Option<ActivePermissionProfile>,
        profile_workspace_roots: Vec<AbsolutePathBuf>,
    },
    LegacyProfile(PermissionProfile),
}

pub fn build_session_settings_apply_plan(
    updates: &SessionSettingsUpdate,
    current: SessionSettingsApplyCurrent<'_>,
    normalized_cwd: AbsolutePathBuf,
    model_options: impl IntoIterator<Item = (impl AsRef<str>, impl AsRef<str>)>,
) -> SessionSettingsApplyPlan {
    let legacy_permission_profile_needs_cwd_rebind = legacy_permission_profile_needs_cwd_rebind(
        current.sandbox_policy,
        current.cwd,
        current.file_system_sandbox_policy,
    );
    let collaboration_mode_updated = updates.collaboration_mode.is_some();
    let collaboration_mode = updates
        .collaboration_mode
        .clone()
        .unwrap_or_else(|| current.collaboration_mode.clone());
    let model_provider_update = model_provider_update_for_collaboration_mode(
        updates.model_provider.clone(),
        collaboration_mode_updated,
        collaboration_mode.model(),
        model_options,
    );
    let model_reasoning_summary = updates.reasoning_summary;
    let service_tier = updates
        .service_tier
        .clone()
        .map(normalize_service_tier_update)
        .unwrap_or_else(|| current.service_tier.clone());
    let personality = updates.personality.or(current.personality);
    let cwd_changed = normalized_cwd.as_path() != current.cwd.as_path();
    let workspace_roots = retarget_workspace_roots_for_cwd_update(
        current.cwd,
        &normalized_cwd,
        current.workspace_roots,
        updates.workspace_roots.clone(),
    );
    let permission_profile_update =
        build_session_permission_profile_update(updates, &current, &normalized_cwd, cwd_changed);
    let app_server_client_name = updates
        .app_server_client_name
        .clone()
        .or_else(|| current.app_server_client_name.clone());
    let app_server_client_version = updates
        .app_server_client_version
        .clone()
        .or_else(|| current.app_server_client_version.clone());

    let permission_profile_update = if permission_profile_update.is_none()
        && cwd_changed
        && legacy_permission_profile_needs_cwd_rebind
    {
        Some(SessionPermissionProfileUpdate::LegacyProfile(
            legacy_permission_profile_for_cwd(
                current.sandbox_policy,
                &normalized_cwd,
                current.file_system_sandbox_policy,
                current.network_sandbox_policy,
            ),
        ))
    } else {
        permission_profile_update
    };

    SessionSettingsApplyPlan {
        collaboration_mode,
        model_provider_update,
        model_reasoning_summary,
        service_tier,
        personality,
        cwd: normalized_cwd,
        workspace_roots,
        permission_profile_update,
        app_server_client_name,
        app_server_client_version,
    }
}

fn build_session_permission_profile_update(
    updates: &SessionSettingsUpdate,
    current: &SessionSettingsApplyCurrent<'_>,
    normalized_cwd: &AbsolutePathBuf,
    cwd_changed: bool,
) -> Option<SessionPermissionProfileUpdate> {
    if let Some(permission_profile) = updates.permission_profile.clone() {
        let active_permission_profile = updates.active_permission_profile.clone().or_else(|| {
            (permission_profile == *current.permission_profile)
                .then(|| current.active_permission_profile.clone())
                .flatten()
        });
        return Some(SessionPermissionProfileUpdate::ActiveProfile {
            permission_profile: permission_profile_preserving_deny_reads(
                &permission_profile,
                Some(current.file_system_sandbox_policy),
            ),
            active_permission_profile,
            profile_workspace_roots: updates.profile_workspace_roots.clone().unwrap_or_default(),
        });
    }

    if let Some(sandbox_policy) = updates.sandbox_policy.clone() {
        return Some(SessionPermissionProfileUpdate::LegacyProfile(
            legacy_permission_profile_for_cwd(
                &sandbox_policy,
                normalized_cwd,
                current.file_system_sandbox_policy,
                NetworkSandboxPolicy::from(&sandbox_policy),
            ),
        ));
    }

    if cwd_changed {
        return None;
    }

    None
}

pub fn model_provider_update_for_collaboration_mode(
    explicit_model_provider: Option<String>,
    collaboration_mode_updated: bool,
    model: &str,
    model_options: impl IntoIterator<Item = (impl AsRef<str>, impl AsRef<str>)>,
) -> Option<String> {
    if explicit_model_provider.is_some() || !collaboration_mode_updated {
        return explicit_model_provider;
    }

    let mut providers = model_options
        .into_iter()
        .filter(|(candidate_model, _)| candidate_model.as_ref() == model)
        .map(|(_, provider)| provider.as_ref().to_string());
    let provider = providers.next()?;
    providers.all(|other| other == provider).then_some(provider)
}

pub fn normalize_service_tier_update(service_tier: Option<String>) -> Option<String> {
    service_tier.map(|service_tier| {
        ServiceTier::from_request_value(&service_tier).map_or(service_tier, |service_tier| {
            service_tier.request_value().to_string()
        })
    })
}

pub fn resolve_session_service_tier(
    configured_service_tier: Option<String>,
    fast_default_opt_out: bool,
    uses_enterprise_default_service_tier: bool,
    fast_mode_enabled: bool,
) -> Option<String> {
    if configured_service_tier.is_some() || fast_default_opt_out || !fast_mode_enabled {
        return configured_service_tier;
    }

    uses_enterprise_default_service_tier.then_some(ServiceTier::Fast.request_value().to_string())
}

pub fn is_enterprise_default_service_tier_plan(plan_type: AccountPlanType) -> bool {
    plan_type == AccountPlanType::Enterprise
        || plan_type.is_business_like()
        || plan_type.is_team_like()
}

/// Computes sticky workspace roots after a session cwd/settings update.
///
/// Explicit roots from the update win. Without explicit roots, a cwd-only
/// update retargets an existing workspace root equal to the old cwd onto the
/// new cwd so project-root permissions keep following the active workspace.
pub fn retarget_workspace_roots_for_cwd_update(
    current_cwd: &AbsolutePathBuf,
    next_cwd: &AbsolutePathBuf,
    current_workspace_roots: &[AbsolutePathBuf],
    updated_workspace_roots: Option<Vec<AbsolutePathBuf>>,
) -> Vec<AbsolutePathBuf> {
    if let Some(workspace_roots) = updated_workspace_roots {
        return workspace_roots;
    }
    if next_cwd.as_path() == current_cwd.as_path() || !current_workspace_roots.contains(current_cwd)
    {
        return current_workspace_roots.to_vec();
    }

    let mut retargeted_workspace_roots = Vec::with_capacity(current_workspace_roots.len());
    for root in current_workspace_roots {
        let root = if root == current_cwd {
            next_cwd.clone()
        } else {
            root.clone()
        };
        if !retargeted_workspace_roots.contains(&root) {
            retargeted_workspace_roots.push(root);
        }
    }
    retargeted_workspace_roots
}

/// Returns true when a cwd-only update should rederive the legacy permission
/// profile against the new cwd.
pub fn legacy_permission_profile_needs_cwd_rebind(
    current_sandbox_policy: &SandboxPolicy,
    current_cwd: &AbsolutePathBuf,
    current_file_system_sandbox_policy: &FileSystemSandboxPolicy,
) -> bool {
    let legacy_file_system_projection =
        FileSystemSandboxPolicy::from_legacy_sandbox_policy_preserving_deny_entries(
            current_sandbox_policy,
            current_cwd,
            current_file_system_sandbox_policy,
        );
    let file_system_policy_matches_legacy = current_file_system_sandbox_policy
        .is_semantically_equivalent_to(&legacy_file_system_projection, current_cwd);
    let file_system_policy_has_rebindable_project_root_write = current_file_system_sandbox_policy
        .entries
        .iter()
        .any(|entry| {
            entry.access.can_write()
                && matches!(
                    &entry.path,
                    FileSystemPath::Special {
                        value: FileSystemSpecialPath::ProjectRoots { subpath: None },
                    }
                )
        });

    file_system_policy_matches_legacy && file_system_policy_has_rebindable_project_root_write
}

/// Builds a permission profile by reprojecting a legacy sandbox policy against
/// a cwd while preserving existing filesystem deny-read restrictions.
pub fn legacy_permission_profile_for_cwd(
    sandbox_policy: &SandboxPolicy,
    cwd: &AbsolutePathBuf,
    preserve_deny_reads_from: &FileSystemSandboxPolicy,
    network_sandbox_policy: NetworkSandboxPolicy,
) -> PermissionProfile {
    let file_system_sandbox_policy =
        FileSystemSandboxPolicy::from_legacy_sandbox_policy_preserving_deny_entries(
            sandbox_policy,
            cwd,
            preserve_deny_reads_from,
        );
    PermissionProfile::from_runtime_permissions_with_enforcement(
        SandboxEnforcement::from_legacy_sandbox_policy(sandbox_policy),
        &file_system_sandbox_policy,
        network_sandbox_policy,
    )
}

/// Builds an active permission-profile projection from a requested profile
/// while preserving deny-read restrictions from the current filesystem policy.
pub fn permission_profile_preserving_deny_reads(
    permission_profile: &PermissionProfile,
    preserve_deny_reads_from: Option<&FileSystemSandboxPolicy>,
) -> PermissionProfile {
    let enforcement = permission_profile.enforcement();
    let (mut file_system_sandbox_policy, network_sandbox_policy) =
        permission_profile.to_runtime_permissions();
    if let Some(existing_file_system_policy) = preserve_deny_reads_from {
        file_system_sandbox_policy
            .preserve_deny_read_restrictions_from(existing_file_system_policy);
    }
    PermissionProfile::from_runtime_permissions_with_enforcement(
        enforcement,
        &file_system_sandbox_policy,
        network_sandbox_policy,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::config_types::ModeKind;
    use protocol::config_types::Settings;
    use protocol::permissions::FileSystemAccessMode;
    use protocol::permissions::FileSystemSandboxEntry;
    use protocol::permissions::FileSystemSandboxKind;
    use protocol::permissions::NetworkSandboxPolicy;

    fn path(value: &str) -> AbsolutePathBuf {
        AbsolutePathBuf::from_absolute_path(value).expect("absolute test path")
    }

    fn project_root_write_policy() -> FileSystemSandboxPolicy {
        FileSystemSandboxPolicy {
            kind: FileSystemSandboxKind::Restricted,
            glob_scan_max_depth: None,
            entries: vec![FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::ProjectRoots { subpath: None },
                },
                access: FileSystemAccessMode::Write,
            }],
        }
    }

    fn collaboration_mode(model: &str) -> CollaborationMode {
        CollaborationMode {
            mode: ModeKind::Default,
            settings: Settings {
                model: model.to_string(),
                reasoning_effort: None,
                developer_instructions: None,
            },
        }
    }

    #[test]
    fn explicit_model_provider_update_wins() {
        let update = model_provider_update_for_collaboration_mode(
            Some("manual".to_string()),
            true,
            "gpt-a",
            [("gpt-a", "auto")],
        );

        assert_eq!(update.as_deref(), Some("manual"));
    }

    #[test]
    fn collaboration_model_update_infers_unique_provider() {
        let update = model_provider_update_for_collaboration_mode(
            None,
            true,
            "gpt-a",
            [("gpt-a", "openai"), ("gpt-b", "other"), ("gpt-a", "openai")],
        );

        assert_eq!(update.as_deref(), Some("openai"));
    }

    #[test]
    fn collaboration_model_update_skips_ambiguous_provider() {
        let update = model_provider_update_for_collaboration_mode(
            None,
            true,
            "gpt-a",
            [("gpt-a", "openai"), ("gpt-a", "azure")],
        );

        assert_eq!(update, None);
    }

    #[test]
    fn service_tier_update_normalizes_legacy_fast_value() {
        assert_eq!(
            normalize_service_tier_update(Some("fast".to_string())).as_deref(),
            Some("priority")
        );
        assert_eq!(
            normalize_service_tier_update(Some("unknown".to_string())).as_deref(),
            Some("unknown")
        );
        assert_eq!(normalize_service_tier_update(None), None);
    }

    #[test]
    fn session_service_tier_defaults_enterprise_accounts_to_fast() {
        assert_eq!(
            resolve_session_service_tier(
                /*configured_service_tier*/ None,
                /*fast_default_opt_out*/ false,
                is_enterprise_default_service_tier_plan(AccountPlanType::Enterprise),
                /*fast_mode_enabled*/ true,
            ),
            Some(ServiceTier::Fast.request_value().to_string())
        );
        assert_eq!(
            resolve_session_service_tier(
                /*configured_service_tier*/ None,
                /*fast_default_opt_out*/ false,
                is_enterprise_default_service_tier_plan(AccountPlanType::EnterpriseCbpUsageBased),
                /*fast_mode_enabled*/ true,
            ),
            Some(ServiceTier::Fast.request_value().to_string())
        );
        assert_eq!(
            resolve_session_service_tier(
                /*configured_service_tier*/ None,
                /*fast_default_opt_out*/ false,
                is_enterprise_default_service_tier_plan(AccountPlanType::Business),
                /*fast_mode_enabled*/ true,
            ),
            Some(ServiceTier::Fast.request_value().to_string())
        );
        assert_eq!(
            resolve_session_service_tier(
                /*configured_service_tier*/ None,
                /*fast_default_opt_out*/ false,
                is_enterprise_default_service_tier_plan(AccountPlanType::Team),
                /*fast_mode_enabled*/ true,
            ),
            Some(ServiceTier::Fast.request_value().to_string())
        );
        assert_eq!(
            resolve_session_service_tier(
                /*configured_service_tier*/ None,
                /*fast_default_opt_out*/ false,
                is_enterprise_default_service_tier_plan(
                    AccountPlanType::SelfServeBusinessUsageBased,
                ),
                /*fast_mode_enabled*/ true,
            ),
            Some(ServiceTier::Fast.request_value().to_string())
        );
    }

    #[test]
    fn session_service_tier_respects_fast_default_opt_out() {
        assert_eq!(
            resolve_session_service_tier(
                /*configured_service_tier*/ None,
                /*fast_default_opt_out*/ true,
                is_enterprise_default_service_tier_plan(AccountPlanType::Enterprise),
                /*fast_mode_enabled*/ true,
            ),
            None
        );
    }

    #[test]
    fn session_service_tier_does_not_default_non_enterprise_or_disabled_fast_mode() {
        assert_eq!(
            resolve_session_service_tier(
                /*configured_service_tier*/ None,
                /*fast_default_opt_out*/ false,
                is_enterprise_default_service_tier_plan(AccountPlanType::Pro),
                /*fast_mode_enabled*/ true,
            ),
            None
        );
        assert_eq!(
            resolve_session_service_tier(
                /*configured_service_tier*/ None,
                /*fast_default_opt_out*/ false,
                is_enterprise_default_service_tier_plan(AccountPlanType::Enterprise),
                /*fast_mode_enabled*/ false,
            ),
            None
        );
    }

    #[test]
    fn retargets_current_cwd_workspace_root_without_duplicates() {
        let old_cwd = path("/tmp/old");
        let new_cwd = path("/tmp/new");
        let other = path("/tmp/other");

        let roots = retarget_workspace_roots_for_cwd_update(
            &old_cwd,
            &new_cwd,
            &[old_cwd.clone(), new_cwd.clone(), other.clone()],
            None,
        );

        assert_eq!(roots, vec![new_cwd, other]);
    }

    #[test]
    fn explicit_workspace_roots_win_over_retargeting() {
        let old_cwd = path("/tmp/old");
        let new_cwd = path("/tmp/new");
        let explicit = vec![path("/tmp/explicit")];

        let roots = retarget_workspace_roots_for_cwd_update(
            &old_cwd,
            &new_cwd,
            std::slice::from_ref(&old_cwd),
            Some(explicit.clone()),
        );

        assert_eq!(roots, explicit);
    }

    #[test]
    fn legacy_profile_rebind_detects_cwd_bound_project_root_write() {
        let cwd = path("/tmp/project");
        let sandbox_policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: Vec::new(),
            network_access: false,
            exclude_tmpdir_env_var: false,
            exclude_slash_tmp: false,
        };
        let file_system_policy =
            FileSystemSandboxPolicy::from_legacy_sandbox_policy_preserving_deny_entries(
                &sandbox_policy,
                &cwd,
                &FileSystemSandboxPolicy::default(),
            );

        assert!(legacy_permission_profile_needs_cwd_rebind(
            &sandbox_policy,
            &cwd,
            &file_system_policy,
        ));
    }

    #[test]
    fn non_legacy_project_root_policy_does_not_rebind() {
        assert!(!legacy_permission_profile_needs_cwd_rebind(
            &SandboxPolicy::DangerFullAccess,
            &path("/tmp/project"),
            &project_root_write_policy(),
        ));
    }

    #[test]
    fn legacy_permission_profile_for_cwd_preserves_network_policy() {
        let cwd = path("/tmp/project");
        let profile = legacy_permission_profile_for_cwd(
            &SandboxPolicy::DangerFullAccess,
            &cwd,
            &FileSystemSandboxPolicy::default(),
            NetworkSandboxPolicy::Enabled,
        );

        assert_eq!(
            profile.network_sandbox_policy(),
            NetworkSandboxPolicy::Enabled
        );
    }

    #[test]
    fn permission_profile_projection_preserves_deny_reads() {
        let base = PermissionProfile::from_runtime_permissions_with_enforcement(
            SandboxEnforcement::Managed,
            &project_root_write_policy(),
            NetworkSandboxPolicy::Restricted,
        );
        let existing = FileSystemSandboxPolicy {
            kind: FileSystemSandboxKind::Restricted,
            glob_scan_max_depth: None,
            entries: vec![FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: path("/tmp/project/secret"),
                },
                access: FileSystemAccessMode::None,
            }],
        };

        let projected = permission_profile_preserving_deny_reads(&base, Some(&existing));

        assert!(
            projected
                .file_system_sandbox_policy()
                .entries
                .iter()
                .any(|entry| entry.access == FileSystemAccessMode::None)
        );
    }

    #[test]
    fn session_settings_apply_plan_projects_update_owned_fields() {
        let old_cwd = path("/tmp/old");
        let new_cwd = path("/tmp/new");
        let active_profile = ActivePermissionProfile::new("dev");
        let permission_profile = PermissionProfile::from_runtime_permissions_with_enforcement(
            SandboxEnforcement::Managed,
            &project_root_write_policy(),
            NetworkSandboxPolicy::Restricted,
        );
        let current_collaboration_mode = collaboration_mode("gpt-a");
        let current_file_system_policy = permission_profile.file_system_sandbox_policy();
        let updates = SessionSettingsUpdate {
            cwd: None,
            workspace_roots: None,
            profile_workspace_roots: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            permission_profile: Some(permission_profile.clone()),
            active_permission_profile: None,
            windows_sandbox_level: None,
            model_provider: None,
            collaboration_mode: Some(collaboration_mode("gpt-b")),
            reasoning_summary: Some(ReasoningSummaryConfig::Detailed),
            service_tier: Some(Some("fast".to_string())),
            final_output_json_schema: None,
            environments: None,
            personality: Some(Personality::Pragmatic),
            app_server_client_name: Some("root-worker".to_string()),
            app_server_client_version: Some("1.2.3".to_string()),
        };

        let plan = build_session_settings_apply_plan(
            &updates,
            SessionSettingsApplyCurrent {
                collaboration_mode: &current_collaboration_mode,
                service_tier: None,
                personality: None,
                cwd: &old_cwd,
                workspace_roots: std::slice::from_ref(&old_cwd),
                permission_profile: &permission_profile,
                active_permission_profile: Some(active_profile.clone()),
                sandbox_policy: &SandboxPolicy::new_workspace_write_policy(),
                file_system_sandbox_policy: &current_file_system_policy,
                network_sandbox_policy: permission_profile.network_sandbox_policy(),
                app_server_client_name: None,
                app_server_client_version: None,
            },
            new_cwd.clone(),
            [("gpt-b", "openai")],
        );

        assert_eq!(plan.collaboration_mode.model(), "gpt-b");
        assert_eq!(plan.model_provider_update.as_deref(), Some("openai"));
        assert_eq!(
            plan.model_reasoning_summary,
            Some(ReasoningSummaryConfig::Detailed)
        );
        assert_eq!(plan.service_tier.as_deref(), Some("priority"));
        assert_eq!(plan.personality, Some(Personality::Pragmatic));
        assert_eq!(plan.cwd, new_cwd.clone());
        assert_eq!(plan.workspace_roots, vec![new_cwd]);
        assert_eq!(plan.app_server_client_name.as_deref(), Some("root-worker"));
        assert_eq!(plan.app_server_client_version.as_deref(), Some("1.2.3"));
        match plan.permission_profile_update {
            Some(SessionPermissionProfileUpdate::ActiveProfile {
                active_permission_profile,
                ..
            }) => assert_eq!(active_permission_profile, Some(active_profile)),
            Some(SessionPermissionProfileUpdate::LegacyProfile(_)) | None => {
                panic!("expected active permission profile update")
            }
        }
    }
}
