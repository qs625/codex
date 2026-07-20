use codex_config_types::Constrained;
use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::config_types::ApprovalsReviewer;
use protocol::config_types::Personality;
use protocol::config_types::ReasoningSummary;
use protocol::models::PermissionProfile;
use protocol::openai_models::ReasoningEffort;
use protocol::protocol::AskForApproval;

use super::Config;
use super::PermissionProfileState;
use super::resolve_web_search_mode_for_turn;

#[derive(Clone)]
/// Session-owned runtime fields that should be layered over the original
/// loaded `Config` when building per-turn configuration.
pub struct SessionConfigOverlay {
    pub cwd: AbsolutePathBuf,
    pub workspace_roots: Vec<AbsolutePathBuf>,
    pub model: String,
    pub model_reasoning_effort: Option<ReasoningEffort>,
    pub model_reasoning_summary: Option<ReasoningSummary>,
    pub service_tier: Option<String>,
    pub personality: Option<Personality>,
    pub approvals_reviewer: ApprovalsReviewer,
    pub permission_profile_state: PermissionProfileState,
}

/// Session overlay plus fields that only belong to the current effective
/// session snapshot, not every per-turn config.
pub struct EffectiveSessionConfigOverlay {
    pub session: SessionConfigOverlay,
    pub model: String,
    pub approval_policy: Constrained<AskForApproval>,
}

pub fn build_per_turn_config_from_session_overlay(
    base_config: &Config,
    overlay: SessionConfigOverlay,
) -> Config {
    let mut per_turn_config = base_config.clone();
    per_turn_config.cwd = overlay.cwd;
    per_turn_config.workspace_roots = overlay.workspace_roots.clone();
    per_turn_config.model = Some(overlay.model);
    per_turn_config
        .permissions
        .set_workspace_roots(overlay.workspace_roots);
    per_turn_config.model_reasoning_effort = overlay.model_reasoning_effort;
    per_turn_config.model_reasoning_summary = overlay.model_reasoning_summary;
    per_turn_config.service_tier = overlay.service_tier;
    per_turn_config.personality = overlay.personality;
    per_turn_config.approvals_reviewer = overlay.approvals_reviewer;
    per_turn_config
        .permissions
        .set_permission_profile_state(overlay.permission_profile_state);

    let permission_profile = per_turn_config.permissions.effective_permission_profile();
    resolve_web_search_mode_for_config(&mut per_turn_config, &permission_profile);
    per_turn_config.features = base_config.features.clone();
    per_turn_config
}

pub fn build_effective_session_config_from_session_overlay(
    base_config: &Config,
    overlay: EffectiveSessionConfigOverlay,
) -> Config {
    let mut config = build_per_turn_config_from_session_overlay(base_config, overlay.session);
    config.model = Some(overlay.model);
    config.permissions.approval_policy = overlay.approval_policy;
    config
}

fn resolve_web_search_mode_for_config(config: &mut Config, permission_profile: &PermissionProfile) {
    let resolved_web_search_mode =
        resolve_web_search_mode_for_turn(&config.web_search_mode, permission_profile);
    if let Err(err) = config.web_search_mode.set(resolved_web_search_mode) {
        let fallback_value = config.web_search_mode.value();
        tracing::warn!(
            error = %err,
            ?resolved_web_search_mode,
            ?fallback_value,
            "resolved web_search_mode is disallowed by requirements; keeping constrained value"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConfigOverrides;
    use crate::config_toml::ConfigToml;
    use protocol::config_types::ModeKind;
    use protocol::config_types::Settings;
    use protocol::models::ActivePermissionProfile;
    use protocol::models::PermissionProfile;
    use tempfile::tempdir;

    fn active_permission_profile_state(
        permission_profile: PermissionProfile,
        profile_id: impl Into<String>,
    ) -> PermissionProfileState {
        PermissionProfileState::from_constrained_active_profile(
            Constrained::allow_any(permission_profile),
            Some(ActivePermissionProfile::new(profile_id)),
            Vec::new(),
        )
        .expect("active permission profile state should be valid")
    }

    fn workspace_root() -> AbsolutePathBuf {
        AbsolutePathBuf::from_absolute_path("/tmp/workspace").expect("absolute test path")
    }

    fn overlay() -> SessionConfigOverlay {
        SessionConfigOverlay {
            cwd: workspace_root(),
            workspace_roots: vec![workspace_root()],
            model: "gpt-test".to_string(),
            model_reasoning_effort: Some(ReasoningEffort::High),
            model_reasoning_summary: Some(ReasoningSummary::Detailed),
            service_tier: Some("priority".to_string()),
            personality: None,
            approvals_reviewer: ApprovalsReviewer::User,
            permission_profile_state: active_permission_profile_state(
                PermissionProfile::workspace_write(),
                "workspace-write",
            ),
        }
    }

    #[tokio::test]
    async fn per_turn_overlay_applies_session_runtime_fields() -> std::io::Result<()> {
        let codex_home = tempdir()?;
        let base_config = Config::load_from_base_config_with_overrides(
            ConfigToml::default(),
            ConfigOverrides::default(),
            AbsolutePathBuf::from_absolute_path(codex_home.path()).expect("tempdir absolute"),
        )
        .await?;

        let config = build_per_turn_config_from_session_overlay(&base_config, overlay());

        assert_eq!(config.cwd, workspace_root());
        assert_eq!(config.workspace_roots, vec![workspace_root()]);
        assert_eq!(config.permissions.workspace_roots(), &[workspace_root()]);
        assert_eq!(config.model.as_deref(), Some("gpt-test"));
        assert_eq!(config.model_reasoning_effort, Some(ReasoningEffort::High));
        assert_eq!(
            config.model_reasoning_summary,
            Some(ReasoningSummary::Detailed)
        );
        assert_eq!(config.service_tier.as_deref(), Some("priority"));
        assert_eq!(
            config.permissions.active_permission_profile(),
            Some(ActivePermissionProfile::new("workspace-write"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn effective_overlay_sets_model_and_approval_policy() -> std::io::Result<()> {
        let codex_home = tempdir()?;
        let base_config = Config::load_from_base_config_with_overrides(
            ConfigToml::default(),
            ConfigOverrides::default(),
            AbsolutePathBuf::from_absolute_path(codex_home.path()).expect("tempdir absolute"),
        )
        .await?;

        let config = build_effective_session_config_from_session_overlay(
            &base_config,
            EffectiveSessionConfigOverlay {
                session: overlay(),
                model: "gpt-test".to_string(),
                approval_policy: Constrained::allow_any(AskForApproval::Never),
            },
        );

        assert_eq!(config.model.as_deref(), Some("gpt-test"));
        assert_eq!(
            config.permissions.approval_policy.value(),
            AskForApproval::Never
        );
        Ok(())
    }

    #[test]
    fn overlay_type_can_be_built_from_collaboration_settings() {
        let collaboration_mode = protocol::config_types::CollaborationMode {
            mode: ModeKind::Default,
            settings: Settings {
                model: "gpt-test".to_string(),
                reasoning_effort: Some(ReasoningEffort::Medium),
                developer_instructions: None,
            },
        };

        let overlay = SessionConfigOverlay {
            model: collaboration_mode.model().to_string(),
            model_reasoning_effort: collaboration_mode.reasoning_effort(),
            ..overlay()
        };

        assert_eq!(
            overlay.model_reasoning_effort,
            Some(ReasoningEffort::Medium)
        );
    }
}
