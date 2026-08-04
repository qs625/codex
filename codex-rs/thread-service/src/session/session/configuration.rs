use super::*;

impl SessionConfiguration {
    pub(crate) fn approval_policy_is_session_override(config: &Config) -> bool {
        config_origin_is_session_flags(config, "approval_policy")
    }

    pub(crate) fn permission_profile_is_session_override(config: &Config) -> bool {
        [
            "sandbox_mode",
            "default_permissions",
            "permissions",
            "sandbox_workspace_write",
        ]
        .into_iter()
        .any(|key_path| config_origin_is_session_flags(config, key_path))
    }

    pub(crate) fn codex_home(&self) -> &AbsolutePathBuf {
        &self.codex_home
    }

    pub(crate) fn permission_profile_state(&self) -> &PermissionProfileState {
        &self.permission_profile_state
    }

    pub(crate) fn permission_profile(&self) -> PermissionProfile {
        self.permission_profile_state
            .permission_profile()
            .clone()
            .materialize_project_roots_with_workspace_roots(&self.workspace_roots)
    }

    pub(crate) fn active_permission_profile(&self) -> Option<ActivePermissionProfile> {
        self.permission_profile_state.active_permission_profile()
    }

    pub(crate) fn profile_workspace_roots(&self) -> &[AbsolutePathBuf] {
        self.permission_profile_state.profile_workspace_roots()
    }

    #[cfg(test)]
    pub(crate) fn set_permission_profile_for_tests(
        &mut self,
        permission_profile: PermissionProfile,
    ) -> ConstraintResult<()> {
        self.permission_profile_state
            .set_legacy_permission_profile(permission_profile)
    }

    pub(crate) fn sandbox_policy(&self) -> SandboxPolicy {
        self.permission_profile()
            .to_legacy_sandbox_policy(&self.cwd)
            .unwrap_or_else(|_| {
                let file_system_sandbox_policy = self.file_system_sandbox_policy();
                codex_sandboxing_api::compatibility_sandbox_policy_for_permission_profile(
                    self.permission_profile_state.permission_profile(),
                    &file_system_sandbox_policy,
                    self.network_sandbox_policy(),
                    &self.cwd,
                )
            })
    }

    pub(crate) fn file_system_sandbox_policy(&self) -> FileSystemSandboxPolicy {
        self.permission_profile().file_system_sandbox_policy()
    }

    pub(crate) fn network_sandbox_policy(&self) -> NetworkSandboxPolicy {
        self.permission_profile_state
            .permission_profile()
            .network_sandbox_policy()
    }

    pub(crate) fn thread_config_snapshot(&self) -> ThreadConfigSnapshot {
        ThreadConfigSnapshot {
            model: self.collaboration_mode.model().to_string(),
            model_provider_id: self.original_config_do_not_use.model_provider_id.clone(),
            service_tier: self.service_tier.clone(),
            approval_policy: self.approval_policy.value(),
            approvals_reviewer: self.approvals_reviewer,
            permission_profile: self.permission_profile(),
            active_permission_profile: self.active_permission_profile(),
            cwd: self.cwd.clone(),
            workspace_roots: self.workspace_roots.clone(),
            profile_workspace_roots: self.profile_workspace_roots().to_vec(),
            ephemeral: self.original_config_do_not_use.ephemeral,
            reasoning_effort: self.collaboration_mode.reasoning_effort(),
            personality: self.personality,
            session_source: self.session_source.clone(),
            root_agent_path: self
                .root_agent_metadata
                .as_ref()
                .and_then(|metadata| metadata.agent_path.as_ref())
                .map(ToString::to_string),
            root_agent_role: self
                .root_agent_metadata
                .as_ref()
                .and_then(|metadata| metadata.agent_role.clone()),
            thread_source: self.thread_source,
        }
    }

    pub(crate) fn apply(&self, updates: &SessionSettingsUpdate) -> ConstraintResult<Self> {
        let mut next_configuration = self.clone();
        let current_sandbox_policy = self.sandbox_policy();
        let current_file_system_sandbox_policy = self.file_system_sandbox_policy();
        let current_network_sandbox_policy = self.network_sandbox_policy();
        let current_permission_profile = self.permission_profile();

        let absolute_cwd = updates
            .cwd
            .as_ref()
            .map(|cwd| {
                AbsolutePathBuf::relative_to_current_dir(normalize_for_native_workdir(
                    cwd.as_path(),
                ))
                .unwrap_or_else(|e| {
                    warn!("failed to normalize update cwd: {cwd:?}: {e}");
                    self.cwd.clone()
                })
            })
            .unwrap_or_else(|| self.cwd.clone());

        let plan = build_session_settings_apply_plan(
            updates,
            SessionSettingsApplyCurrent {
                collaboration_mode: &self.collaboration_mode,
                service_tier: self.service_tier.clone(),
                personality: self.personality,
                cwd: &self.cwd,
                workspace_roots: &self.workspace_roots,
                permission_profile: &current_permission_profile,
                active_permission_profile: self.active_permission_profile(),
                sandbox_policy: &current_sandbox_policy,
                file_system_sandbox_policy: &current_file_system_sandbox_policy,
                network_sandbox_policy: current_network_sandbox_policy,
                app_server_client_name: self.app_server_client_name.clone(),
                app_server_client_version: self.app_server_client_version.clone(),
            },
            absolute_cwd,
            next_configuration
                .original_config_do_not_use
                .model_options
                .iter()
                .map(|model_option| (model_option.model.as_str(), model_option.provider.as_str())),
        );

        next_configuration.collaboration_mode = plan.collaboration_mode;
        if let Some(model_provider_id) = plan.model_provider_update {
            let Some(model_provider) = next_configuration
                .original_config_do_not_use
                .model_providers
                .get(&model_provider_id)
                .cloned()
            else {
                let allowed = next_configuration
                    .original_config_do_not_use
                    .model_providers
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(ConstraintError::InvalidValue {
                    field_name: "model_provider",
                    candidate: model_provider_id,
                    allowed,
                    requirement_source: RequirementSource::Unknown,
                });
            };
            let mut config = (*next_configuration.original_config_do_not_use).clone();
            config.model_provider_id = model_provider_id;
            config.model_provider = model_provider.clone();
            next_configuration.original_config_do_not_use = Arc::new(config);
            next_configuration.provider = model_provider;
        }
        if let Some(summary) = plan.model_reasoning_summary {
            next_configuration.model_reasoning_summary = Some(summary);
        }
        next_configuration.service_tier = plan.service_tier;
        next_configuration.personality = plan.personality;
        if let Some(approval_policy) = updates.approval_policy {
            next_configuration.approval_policy.set(approval_policy)?;
            next_configuration.approval_policy_is_session_override = true;
        }
        if let Some(approvals_reviewer) = updates.approvals_reviewer {
            next_configuration.approvals_reviewer = approvals_reviewer;
        }
        if let Some(windows_sandbox_level) = updates.windows_sandbox_level {
            next_configuration.windows_sandbox_level = windows_sandbox_level;
        }

        next_configuration.cwd = plan.cwd;
        next_configuration.workspace_roots = plan.workspace_roots;
        if let Some(permission_profile_update) = plan.permission_profile_update {
            next_configuration.permission_profile_is_session_override = true;
            match permission_profile_update {
                SessionPermissionProfileUpdate::ActiveProfile {
                    permission_profile,
                    active_permission_profile,
                    profile_workspace_roots,
                } => next_configuration
                    .permission_profile_state
                    .set_active_permission_profile(
                        permission_profile,
                        active_permission_profile,
                        profile_workspace_roots,
                    )?,
                SessionPermissionProfileUpdate::LegacyProfile(permission_profile) => {
                    next_configuration
                        .permission_profile_state
                        .set_legacy_permission_profile(permission_profile)?;
                }
            }
        }
        next_configuration.app_server_client_name = plan.app_server_client_name;
        next_configuration.app_server_client_version = plan.app_server_client_version;
        Ok(next_configuration)
    }
}

fn config_origin_is_session_flags(config: &Config, key_path: &str) -> bool {
    config
        .config_layer_stack
        .origins()
        .get(key_path)
        .is_some_and(|origin| {
            matches!(origin.name, config_service::ConfigLayerSource::SessionFlags)
        })
}
