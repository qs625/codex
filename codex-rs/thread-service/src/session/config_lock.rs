use crate::ConfigLockBuildInput;
use crate::ConfigLockMultiAgentV2ResolvedConfig;
use crate::ConfigLockResolvedConfigFields;
use crate::ConfigLockSessionResolvedFields;
use crate::build_config_lockfile_toml;
use crate::config_lock_to_pretty_toml;
use anyhow::Context;
use codex_config_toml::ConfigLockReplayOptions;
use codex_config_toml::config_toml::ConfigToml;
use codex_config_toml::validate_config_lock_replay;
use codex_config_types::ConfigLockfileToml;
use protocol::ThreadId;

use super::SessionConfiguration;

pub(crate) async fn validate_config_lock_if_configured(
    session_configuration: &SessionConfiguration,
) -> anyhow::Result<()> {
    if session_configuration.session_source.is_non_root_agent() {
        return Ok(());
    }
    let Some(expected) = session_configuration
        .original_config_do_not_use
        .config_lock_toml
        .as_ref()
    else {
        return Ok(());
    };
    let actual = session_configuration.to_config_lockfile_toml()?;
    let config = session_configuration.original_config_do_not_use.as_ref();
    let options = ConfigLockReplayOptions {
        allow_codex_version_mismatch: config.config_lock_allow_codex_version_mismatch,
    };
    validate_config_lock_replay(expected, &actual, options)
        .context("config lock replay validation failed")?;
    Ok(())
}

pub(crate) async fn export_config_lock_if_configured(
    session_configuration: &SessionConfiguration,
    conversation_id: ThreadId,
) -> anyhow::Result<()> {
    let config = session_configuration.original_config_do_not_use.as_ref();
    let Some(export_dir) = config.config_lock_export_dir.as_ref() else {
        return Ok(());
    };

    let lock = session_configuration.to_config_lockfile_toml()?;
    let lock = config_lock_to_pretty_toml(&lock)?;
    let path = export_dir.join(format!("{conversation_id}.config.lock.toml"));

    tokio::fs::create_dir_all(export_dir)
        .await
        .with_context(|| {
            format!(
                "failed to create config lock export directory {}",
                export_dir.display()
            )
        })?;
    tokio::fs::write(&path, lock)
        .await
        .with_context(|| format!("failed to write config lock to {}", path.display()))?;

    Ok(())
}

impl SessionConfiguration {
    pub(crate) fn to_config_lockfile_toml(&self) -> anyhow::Result<ConfigLockfileToml<ConfigToml>> {
        build_config_lockfile_toml(config_lock_build_input(self)?)
    }
}

fn config_lock_build_input(sc: &SessionConfiguration) -> anyhow::Result<ConfigLockBuildInput> {
    let config = sc.original_config_do_not_use.as_ref();
    let effective_config: ConfigToml = config
        .config_layer_stack
        .effective_config()
        .try_into()
        .context("failed to deserialize effective config for config lock")?;

    Ok(ConfigLockBuildInput {
        effective_config,
        save_fields_resolved_from_model_catalog: config
            .config_lock_save_fields_resolved_from_model_catalog,
        session: ConfigLockSessionResolvedFields {
            model: sc.collaboration_mode.model().to_string(),
            model_reasoning_effort: sc.collaboration_mode.reasoning_effort(),
            model_reasoning_summary: sc.model_reasoning_summary,
            service_tier: sc.service_tier.clone(),
            instructions: sc.base_instructions.clone(),
            developer_instructions: sc.developer_instructions.clone(),
            compact_prompt: sc.compact_prompt.clone(),
            personality: sc.personality,
            approval_policy: sc.approval_policy.value(),
            approvals_reviewer: sc.approvals_reviewer,
        },
        config: ConfigLockResolvedConfigFields {
            web_search: config.web_search_mode.value(),
            model_provider: config.model_provider_id.clone(),
            plan_mode_reasoning_effort: config.plan_mode_reasoning_effort,
            model_verbosity: config.model_verbosity,
            include_permissions_instructions: config.include_permissions_instructions,
            include_apps_instructions: config.include_apps_instructions,
            include_collaboration_mode_instructions: config.include_collaboration_mode_instructions,
            include_environment_context: config.include_environment_context,
            background_terminal_max_timeout: config.background_terminal_max_timeout,
            features: config.features.get().clone(),
            multi_agent_v2: ConfigLockMultiAgentV2ResolvedConfig {
                max_concurrent_threads_per_session: config
                    .multi_agent_v2
                    .max_concurrent_threads_per_session,
                min_wait_timeout_ms: config.multi_agent_v2.min_wait_timeout_ms,
                max_wait_timeout_ms: config.multi_agent_v2.max_wait_timeout_ms,
                default_wait_timeout_ms: config.multi_agent_v2.default_wait_timeout_ms,
                usage_hint_enabled: config.multi_agent_v2.usage_hint_enabled,
                usage_hint_text: config.multi_agent_v2.usage_hint_text.clone(),
                root_agent_usage_hint_text: config
                    .multi_agent_v2
                    .root_agent_usage_hint_text
                    .clone(),
                subagent_usage_hint_text: config.multi_agent_v2.subagent_usage_hint_text.clone(),
                hide_spawn_agent_metadata: config.multi_agent_v2.hide_spawn_agent_metadata,
                non_code_mode_only: config.multi_agent_v2.non_code_mode_only,
            },
            apps_mcp_path_override: config.apps_mcp_path_override.clone(),
            memories: config.memories.clone(),
            agent_max_depth: config.agent_max_depth,
            agent_job_max_runtime_seconds: config.agent_job_max_runtime_seconds,
            agent_interrupt_message_enabled: config.agent_interrupt_message_enabled,
            include_skill_instructions: config.include_skill_instructions,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_config_toml::CONFIG_LOCK_VERSION;
    use codex_features::FeatureToml;
    use codex_features::MultiAgentV2ConfigToml;
    use pretty_assertions::assert_eq;
    use std::sync::Arc;

    #[tokio::test]
    async fn lock_contains_prompts_and_materializes_features() {
        let mut sc = crate::session::tests::make_session_configuration_for_tests().await;
        sc.base_instructions = "resolved instructions".to_string();
        sc.developer_instructions = Some("resolved developer instructions".to_string());
        sc.compact_prompt = Some("resolved compact prompt".to_string());

        let lockfile = sc.to_config_lockfile_toml().expect("lock should serialize");
        let lock = &lockfile.config;

        assert_eq!(lock.instructions, Some(sc.base_instructions.clone()));
        assert_eq!(lock.developer_instructions, sc.developer_instructions);
        assert_eq!(lock.compact_prompt, sc.compact_prompt);
        assert_eq!(lock.model, Some(sc.collaboration_mode.model().to_string()));
        assert_eq!(
            lock.model_reasoning_effort,
            sc.collaboration_mode.reasoning_effort()
        );
        assert_eq!(lock.profile, None);
        assert!(lock.profiles.is_empty());
        assert!(
            lock.debug
                .as_ref()
                .is_none_or(|debug| debug.config_lockfile.is_none())
        );
        assert!(lock.memories.is_some());

        let features = lock
            .features
            .as_ref()
            .expect("lock should materialize feature states");
        let feature_entries = features.entries();
        for spec in codex_features::FEATURES {
            assert_eq!(
                feature_entries.get(spec.key),
                Some(&sc.original_config_do_not_use.features.enabled(spec.id)),
                "{}",
                spec.key
            );
        }

        let multi_agent_v2 = features
            .multi_agent_v2
            .as_ref()
            .expect("multi_agent_v2 config should be materialized");
        assert!(matches!(
            multi_agent_v2,
            FeatureToml::Config(MultiAgentV2ConfigToml {
                enabled: Some(false),
                max_concurrent_threads_per_session: Some(_),
                min_wait_timeout_ms: Some(_),
                max_wait_timeout_ms: Some(_),
                default_wait_timeout_ms: Some(_),
                usage_hint_enabled: Some(_),
                hide_spawn_agent_metadata: Some(_),
                ..
            })
        ));

        assert_eq!(lockfile.version, CONFIG_LOCK_VERSION);
    }

    #[tokio::test]
    async fn lock_skips_session_values_when_model_catalog_fields_are_not_saved() {
        let mut sc = crate::session::tests::make_session_configuration_for_tests().await;
        let mut config = (*sc.original_config_do_not_use).clone();
        config.config_lock_save_fields_resolved_from_model_catalog = false;
        sc.original_config_do_not_use = Arc::new(config);
        sc.base_instructions = "catalog instructions".to_string();
        sc.developer_instructions = Some("catalog developer instructions".to_string());
        sc.compact_prompt = Some("catalog compact prompt".to_string());
        sc.service_tier = Some("flex".to_string());

        let lockfile = sc.to_config_lockfile_toml().expect("lock should serialize");
        let lock = &lockfile.config;

        assert_eq!(lock.model, None);
        assert_eq!(lock.model_reasoning_effort, None);
        assert_eq!(lock.model_reasoning_summary, None);
        assert_eq!(lock.service_tier, None);
        assert_eq!(lock.instructions, None);
        assert_eq!(lock.developer_instructions, None);
        assert_eq!(lock.compact_prompt, None);
        assert_eq!(lock.personality, None);
        assert_eq!(lock.approval_policy, None);
        assert_eq!(lock.approvals_reviewer, None);
    }

    #[tokio::test]
    async fn lock_validation_reports_config_diff() {
        let sc = crate::session::tests::make_session_configuration_for_tests().await;
        let expected = sc.to_config_lockfile_toml().expect("lock should serialize");
        let mut actual = expected.clone();
        actual.config.model = Some("different-model".to_string());

        let error =
            validate_config_lock_replay(&expected, &actual, ConfigLockReplayOptions::default())
                .expect_err("config drift should fail");
        let message = error.to_string();
        assert!(
            message.contains("replayed effective config does not match config lock"),
            "{message}"
        );
        assert!(message.contains("model = "), "{message}");
    }

    #[tokio::test]
    async fn lock_validation_rejects_codex_version_mismatch_by_default() {
        let sc = crate::session::tests::make_session_configuration_for_tests().await;
        let mut expected = sc.to_config_lockfile_toml().expect("lock should serialize");
        expected.codex_version = "older-version".to_string();
        let actual = sc.to_config_lockfile_toml().expect("lock should serialize");

        let error =
            validate_config_lock_replay(&expected, &actual, ConfigLockReplayOptions::default())
                .expect_err("version drift should fail");
        let message = error.to_string();
        assert!(
            message.contains("config lock Codex version mismatch"),
            "{message}"
        );
        assert!(
            message.contains("debug.config_lockfile.allow_codex_version_mismatch=true"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn lock_validation_can_ignore_codex_version_mismatch() {
        let sc = crate::session::tests::make_session_configuration_for_tests().await;
        let mut expected = sc.to_config_lockfile_toml().expect("lock should serialize");
        expected.codex_version = "older-version".to_string();
        let actual = sc.to_config_lockfile_toml().expect("lock should serialize");

        validate_config_lock_replay(
            &expected,
            &actual,
            ConfigLockReplayOptions {
                allow_codex_version_mismatch: true,
            },
        )
        .expect("version drift should be ignored");
    }
}
