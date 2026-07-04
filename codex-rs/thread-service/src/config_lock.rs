use anyhow::Context;
use codex_config_toml::clear_config_lock_debug_controls;
use codex_config_toml::config_lockfile;
use codex_config_toml::config_toml::ConfigToml;
use codex_config_toml::toml_round_trip;
use codex_config_types::ConfigLockfileToml;
use codex_config_types::MemoriesConfig;
use codex_config_types::MemoriesToml;
use codex_features::AppsMcpPathOverrideConfigToml;
use codex_features::Feature;
use codex_features::Features;
use codex_features::FeaturesToml;
use codex_features::MultiAgentV2ConfigToml;
use protocol::config_types::ApprovalsReviewer;
use protocol::config_types::Personality;
use protocol::config_types::ReasoningSummary;
use protocol::config_types::Verbosity;
use protocol::config_types::WebSearchMode;
use protocol::openai_models::ReasoningEffort;
use protocol::protocol::AskForApproval;
use serde::Serialize;

/// Resolved session fields that may not be present in the raw config layer stack.
#[derive(Clone, Debug)]
pub struct ConfigLockSessionResolvedFields {
    pub model: String,
    pub model_reasoning_effort: Option<ReasoningEffort>,
    pub model_reasoning_summary: Option<ReasoningSummary>,
    pub service_tier: Option<String>,
    pub instructions: String,
    pub developer_instructions: Option<String>,
    pub compact_prompt: Option<String>,
    pub personality: Option<Personality>,
    pub approval_policy: AskForApproval,
    pub approvals_reviewer: ApprovalsReviewer,
}

/// Resolved runtime config fields that should be persisted in replayable form.
#[derive(Clone, Debug)]
pub struct ConfigLockResolvedConfigFields {
    pub web_search: WebSearchMode,
    pub model_provider: String,
    pub plan_mode_reasoning_effort: Option<ReasoningEffort>,
    pub model_verbosity: Option<Verbosity>,
    pub include_permissions_instructions: bool,
    pub include_apps_instructions: bool,
    pub include_collaboration_mode_instructions: bool,
    pub include_environment_context: bool,
    pub background_terminal_max_timeout: u64,
    pub features: Features,
    pub multi_agent_v2: ConfigLockMultiAgentV2ResolvedConfig,
    pub apps_mcp_path_override: Option<String>,
    pub memories: MemoriesConfig,
    pub agent_max_depth: i32,
    pub agent_job_max_runtime_seconds: Option<u64>,
    pub agent_interrupt_message_enabled: bool,
    pub include_skill_instructions: bool,
}

/// Effective MultiAgent V2 settings in the shape needed for lockfile replay.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ConfigLockMultiAgentV2ResolvedConfig {
    pub max_concurrent_threads_per_session: usize,
    pub min_wait_timeout_ms: i64,
    pub max_wait_timeout_ms: i64,
    pub default_wait_timeout_ms: i64,
    pub usage_hint_enabled: bool,
    pub usage_hint_text: Option<String>,
    pub root_agent_usage_hint_text: Option<String>,
    pub subagent_usage_hint_text: Option<String>,
    pub hide_spawn_agent_metadata: bool,
    pub non_code_mode_only: bool,
}

/// Fully resolved input needed to build a config lockfile without depending on core.
#[derive(Clone, Debug)]
pub struct ConfigLockBuildInput {
    pub effective_config: ConfigToml,
    pub save_fields_resolved_from_model_catalog: bool,
    pub session: ConfigLockSessionResolvedFields,
    pub config: ConfigLockResolvedConfigFields,
}

pub fn build_config_lockfile_toml(
    input: ConfigLockBuildInput,
) -> anyhow::Result<ConfigLockfileToml<ConfigToml>> {
    Ok(config_lockfile(build_config_lock_toml(input)?))
}

fn build_config_lock_toml(input: ConfigLockBuildInput) -> anyhow::Result<ConfigToml> {
    let ConfigLockBuildInput {
        mut effective_config,
        save_fields_resolved_from_model_catalog,
        session,
        config,
    } = input;

    if save_fields_resolved_from_model_catalog {
        save_session_resolved_fields(&session, &mut effective_config);
    }

    save_config_resolved_fields(&config, &mut effective_config)?;
    drop_lockfile_inputs(&mut effective_config);

    Ok(effective_config)
}

/// Saves values chosen during session construction from the model catalog,
/// collaboration mode, and resolved prompt setup.
fn save_session_resolved_fields(
    session: &ConfigLockSessionResolvedFields,
    lock_config: &mut ConfigToml,
) {
    lock_config.model = Some(session.model.clone());
    lock_config.model_reasoning_effort = session.model_reasoning_effort;
    lock_config.model_reasoning_summary = session.model_reasoning_summary;
    lock_config.service_tier = session.service_tier.clone();
    lock_config.instructions = Some(session.instructions.clone());
    lock_config.developer_instructions = session.developer_instructions.clone();
    lock_config.compact_prompt = session.compact_prompt.clone();
    lock_config.personality = session.personality;
    lock_config.approval_policy = Some(session.approval_policy);
    lock_config.approvals_reviewer = Some(session.approvals_reviewer);
}

/// Saves values stored after higher-level resolution, normalization, defaulting,
/// or feature materialization.
fn save_config_resolved_fields(
    config: &ConfigLockResolvedConfigFields,
    lock_config: &mut ConfigToml,
) -> anyhow::Result<()> {
    lock_config.web_search = Some(config.web_search);
    lock_config.model_provider = Some(config.model_provider.clone());
    lock_config.plan_mode_reasoning_effort = config.plan_mode_reasoning_effort;
    lock_config.model_verbosity = config.model_verbosity;
    lock_config.include_permissions_instructions = Some(config.include_permissions_instructions);
    lock_config.include_apps_instructions = Some(config.include_apps_instructions);
    lock_config.include_collaboration_mode_instructions =
        Some(config.include_collaboration_mode_instructions);
    lock_config.include_environment_context = Some(config.include_environment_context);
    lock_config.background_terminal_max_timeout = Some(config.background_terminal_max_timeout);

    let features = lock_config
        .features
        .get_or_insert_with(FeaturesToml::default);
    features.materialize_resolved_enabled(&config.features);
    let mut multi_agent_v2: MultiAgentV2ConfigToml =
        resolved_config_to_toml(&config.multi_agent_v2, "features.multi_agent_v2")?;
    multi_agent_v2.enabled = Some(config.features.enabled(Feature::MultiAgentV2));
    features.multi_agent_v2 = Some(codex_features::FeatureToml::Config(multi_agent_v2));
    features.apps_mcp_path_override = Some(codex_features::FeatureToml::Config(
        AppsMcpPathOverrideConfigToml {
            enabled: Some(config.features.enabled(Feature::AppsMcpPathOverride)),
            path: config.apps_mcp_path_override.clone(),
        },
    ));
    lock_config.memories = Some(resolved_config_to_toml::<MemoriesToml>(
        &config.memories,
        "memories",
    )?);

    let agents = lock_config.agents.get_or_insert_with(Default::default);
    agents.max_threads = None;
    agents.max_depth = Some(config.agent_max_depth);
    agents.job_max_runtime_seconds = config.agent_job_max_runtime_seconds;
    agents.interrupt_message = Some(config.agent_interrupt_message_enabled);

    lock_config
        .skills
        .get_or_insert_with(Default::default)
        .include_instructions = Some(config.include_skill_instructions);

    Ok(())
}

fn drop_lockfile_inputs(lock_config: &mut ConfigToml) {
    lock_config.profile = None;
    lock_config.profiles.clear();
    clear_config_lock_debug_controls(lock_config);
    lock_config.model_instructions_file = None;
    lock_config.experimental_compact_prompt_file = None;
    lock_config.model_catalog_json = None;
    lock_config.sandbox_mode = None;
    lock_config.sandbox_workspace_write = None;
    lock_config.default_permissions = None;
    lock_config.permissions = None;
    lock_config.experimental_use_unified_exec_tool = None;
}

fn resolved_config_to_toml<Toml>(
    value: &impl serde::Serialize,
    label: &'static str,
) -> anyhow::Result<Toml>
where
    Toml: serde::de::DeserializeOwned + serde::Serialize,
{
    toml_round_trip(value, label).map_err(anyhow::Error::from)
}

pub fn config_lock_to_pretty_toml(lock: &ConfigLockfileToml<ConfigToml>) -> anyhow::Result<String> {
    toml::to_string_pretty(lock).context("failed to serialize config lock")
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_features::Feature;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use protocol::config_types::ReasoningSummary;
    use protocol::config_types::Verbosity;
    use protocol::openai_models::ReasoningEffort;

    fn sample_input(save_session_fields: bool) -> ConfigLockBuildInput {
        let mut effective_config = ConfigToml {
            profile: Some("dev".to_string()),
            model_instructions_file: Some(
                AbsolutePathBuf::from_absolute_path("/tmp/instructions.md")
                    .expect("absolute test path"),
            ),
            experimental_use_unified_exec_tool: Some(true),
            ..Default::default()
        };
        effective_config
            .agents
            .get_or_insert_with(Default::default)
            .max_threads = Some(8);

        let mut features = Features::with_defaults();
        features.enable(Feature::MultiAgentV2);
        features.enable(Feature::AppsMcpPathOverride);

        ConfigLockBuildInput {
            effective_config,
            save_fields_resolved_from_model_catalog: save_session_fields,
            session: ConfigLockSessionResolvedFields {
                model: "gpt-test".to_string(),
                model_reasoning_effort: Some(ReasoningEffort::High),
                model_reasoning_summary: Some(ReasoningSummary::Detailed),
                service_tier: Some("priority".to_string()),
                instructions: "base".to_string(),
                developer_instructions: Some("dev".to_string()),
                compact_prompt: Some("compact".to_string()),
                personality: None,
                approval_policy: AskForApproval::OnRequest,
                approvals_reviewer: ApprovalsReviewer::User,
            },
            config: ConfigLockResolvedConfigFields {
                web_search: WebSearchMode::Cached,
                model_provider: "openai".to_string(),
                plan_mode_reasoning_effort: Some(ReasoningEffort::Medium),
                model_verbosity: Some(Verbosity::High),
                include_permissions_instructions: true,
                include_apps_instructions: true,
                include_collaboration_mode_instructions: true,
                include_environment_context: true,
                background_terminal_max_timeout: 30_000,
                features,
                multi_agent_v2: ConfigLockMultiAgentV2ResolvedConfig {
                    max_concurrent_threads_per_session: 4,
                    min_wait_timeout_ms: 100,
                    max_wait_timeout_ms: 1_000,
                    default_wait_timeout_ms: 500,
                    usage_hint_enabled: false,
                    usage_hint_text: Some("hint".to_string()),
                    root_agent_usage_hint_text: None,
                    subagent_usage_hint_text: None,
                    hide_spawn_agent_metadata: true,
                    non_code_mode_only: true,
                },
                apps_mcp_path_override: Some("/tmp/apps-mcp".to_string()),
                memories: MemoriesConfig::default(),
                agent_max_depth: 3,
                agent_job_max_runtime_seconds: Some(60),
                agent_interrupt_message_enabled: true,
                include_skill_instructions: true,
            },
        }
    }

    #[test]
    fn lock_materializes_resolved_fields_and_drops_non_replay_inputs() {
        let lockfile =
            build_config_lockfile_toml(sample_input(/*save_session_fields*/ true)).unwrap();
        let lock = lockfile.config;

        assert_eq!(lock.model.as_deref(), Some("gpt-test"));
        assert_eq!(lock.model_provider.as_deref(), Some("openai"));
        assert_eq!(lock.instructions.as_deref(), Some("base"));
        assert_eq!(lock.developer_instructions.as_deref(), Some("dev"));
        assert_eq!(lock.compact_prompt.as_deref(), Some("compact"));
        assert_eq!(lock.web_search, Some(WebSearchMode::Cached));
        assert_eq!(lock.model_reasoning_effort, Some(ReasoningEffort::High));
        assert_eq!(
            lock.plan_mode_reasoning_effort,
            Some(ReasoningEffort::Medium)
        );
        assert_eq!(lock.model_verbosity, Some(Verbosity::High));
        assert_eq!(lock.background_terminal_max_timeout, Some(30_000));
        assert_eq!(lock.profile, None);
        assert!(lock.profiles.is_empty());
        assert_eq!(lock.model_instructions_file, None);
        assert_eq!(lock.experimental_use_unified_exec_tool, None);

        let features = lock.features.expect("features should be materialized");
        assert!(features.entries().contains_key(Feature::MultiAgentV2.key()));
        assert!(matches!(
            features.multi_agent_v2,
            Some(codex_features::FeatureToml::Config(
                MultiAgentV2ConfigToml {
                    enabled: Some(true),
                    max_concurrent_threads_per_session: Some(4),
                    min_wait_timeout_ms: Some(100),
                    max_wait_timeout_ms: Some(1_000),
                    default_wait_timeout_ms: Some(500),
                    usage_hint_enabled: Some(false),
                    hide_spawn_agent_metadata: Some(true),
                    non_code_mode_only: Some(true),
                    ..
                }
            ))
        ));
        assert!(matches!(
            features.apps_mcp_path_override,
            Some(codex_features::FeatureToml::Config(
                AppsMcpPathOverrideConfigToml {
                    enabled: Some(true),
                    path: Some(path),
                }
            )) if path == "/tmp/apps-mcp"
        ));

        let agents = lock.agents.expect("agents should be materialized");
        assert_eq!(agents.max_threads, None);
        assert_eq!(agents.max_depth, Some(3));
        assert_eq!(agents.job_max_runtime_seconds, Some(60));
        assert_eq!(agents.interrupt_message, Some(true));
        assert_eq!(
            lock.skills.and_then(|skills| skills.include_instructions),
            Some(true)
        );
    }

    #[test]
    fn lock_can_skip_model_catalog_session_fields() {
        let lockfile =
            build_config_lockfile_toml(sample_input(/*save_session_fields*/ false)).unwrap();
        let lock = lockfile.config;

        assert_eq!(lock.model, None);
        assert_eq!(lock.model_reasoning_effort, None);
        assert_eq!(lock.model_reasoning_summary, None);
        assert_eq!(lock.service_tier, None);
        assert_eq!(lock.instructions, None);
        assert_eq!(lock.developer_instructions, None);
        assert_eq!(lock.compact_prompt, None);
        assert_eq!(lock.approval_policy, None);
        assert_eq!(lock.approvals_reviewer, None);
        assert_eq!(lock.model_provider.as_deref(), Some("openai"));
    }
}
