use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use codex_config_types::McpServerConfig;
use codex_context_manager::ContextualUserFragment;
use codex_context_manager::GuardianFollowupReviewReminder;
use codex_features::Feature;
use codex_guardian::GuardianPromptItems;
use codex_guardian::GuardianPromptMode;
use codex_guardian::GuardianReviewForkSnapshot;
use codex_guardian::GuardianReviewModelInfo;
use codex_guardian::GuardianReviewSessionHost;
pub(crate) use codex_guardian::GuardianReviewSessionOutcome;
use codex_guardian::GuardianReviewSpawnKind;
use codex_guardian::build_guardian_prompt_items_from_entries;
use codex_guardian::collect_guardian_transcript_entries;
use codex_guardian::guardian_policy_prompt;
use codex_guardian::guardian_policy_prompt_with_config;
use codex_model_provider_info::ModelProviderInfo;
use codex_network_proxy_api::NetworkProxyConfig;
use codex_protocol::config_types::Personality;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::protocol::SubAgentSource;
use codex_protocol::protocol::TokenUsage;
use codex_protocol::user_input::UserInput;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::codex_delegate::run_codex_thread_interactive;
use crate::config::Config;
use crate::config::Constrained;
use crate::config::ManagedFeatures;
use crate::config::NetworkProxySpec;
use crate::config::Permissions;
use crate::session::Codex;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;

use approval_service::guardian::GUARDIAN_REVIEWER_NAME;
use codex_guardian::GuardianApprovalRequest;

pub(crate) type GuardianReviewSessionManager =
    codex_guardian::GuardianReviewSessionManager<Session>;

#[derive(Clone)]
pub struct GuardianReviewSessionRequest {
    pub(crate) parent_session: Arc<Session>,
    pub(crate) parent_turn: Arc<TurnContext>,
    pub(crate) spawn_config: Config,
    pub(crate) model: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuardianReviewSessionReuseKey {
    // Only include settings that affect spawned-session behavior so reuse
    // invalidation remains explicit and does not depend on unrelated config
    // bookkeeping.
    model: Option<String>,
    model_provider_id: String,
    model_provider: ModelProviderInfo,
    model_context_window: Option<i64>,
    model_auto_compact_token_limit: Option<i64>,
    model_reasoning_effort: Option<ReasoningEffortConfig>,
    model_reasoning_summary: Option<ReasoningSummaryConfig>,
    permissions: Permissions,
    developer_instructions: Option<String>,
    base_instructions: Option<String>,
    user_instructions: Option<String>,
    compact_prompt: Option<String>,
    cwd: AbsolutePathBuf,
    mcp_servers: Constrained<HashMap<String, McpServerConfig>>,
    codex_linux_sandbox_exe: Option<PathBuf>,
    main_execve_wrapper_exe: Option<PathBuf>,
    zsh_path: Option<PathBuf>,
    features: ManagedFeatures,
    use_experimental_unified_exec_tool: bool,
}

impl GuardianReviewSessionReuseKey {
    pub(crate) fn from_spawn_config(spawn_config: &Config) -> Self {
        Self {
            model: spawn_config.model.clone(),
            model_provider_id: spawn_config.model_provider_id.clone(),
            model_provider: spawn_config.model_provider.clone(),
            model_context_window: spawn_config.model_context_window,
            model_auto_compact_token_limit: spawn_config.model_auto_compact_token_limit,
            model_reasoning_effort: spawn_config.model_reasoning_effort,
            model_reasoning_summary: spawn_config.model_reasoning_summary,
            permissions: spawn_config.permissions.clone(),
            developer_instructions: spawn_config.developer_instructions.clone(),
            base_instructions: spawn_config.base_instructions.clone(),
            user_instructions: spawn_config.user_instructions.clone(),
            compact_prompt: spawn_config.compact_prompt.clone(),
            cwd: spawn_config.cwd.clone(),
            mcp_servers: spawn_config.mcp_servers.clone(),
            codex_linux_sandbox_exe: spawn_config.codex_linux_sandbox_exe.clone(),
            main_execve_wrapper_exe: spawn_config.main_execve_wrapper_exe.clone(),
            zsh_path: spawn_config.zsh_path.clone(),
            features: spawn_config.features.clone(),
            use_experimental_unified_exec_tool: spawn_config.use_experimental_unified_exec_tool,
        }
    }
}

impl GuardianReviewSessionHost for Session {
    type Request = GuardianReviewSessionRequest;
    type ReuseKey = GuardianReviewSessionReuseKey;
    type Session = Codex;

    async fn spawn_session(
        &self,
        request: &Self::Request,
        spawn_kind: GuardianReviewSpawnKind,
        cancel_token: CancellationToken,
        fork_snapshot: Option<GuardianReviewForkSnapshot>,
    ) -> anyhow::Result<Self::Session> {
        let mut spawn_config = request.spawn_config.clone();
        if matches!(spawn_kind, GuardianReviewSpawnKind::Ephemeral) {
            spawn_config.ephemeral = true;
        }
        let initial_history = fork_snapshot.map(|snapshot| snapshot.initial_history);
        run_codex_thread_interactive(
            spawn_config,
            self.services.auth_runtime.clone(),
            self.services.model_client.auth_manager(),
            self.services.models_manager.clone(),
            Arc::clone(&request.parent_session),
            Arc::clone(&request.parent_turn),
            cancel_token,
            SubAgentSource::Other(GUARDIAN_REVIEWER_NAME.to_string()),
            initial_history,
        )
        .await
        .map_err(Into::into)
    }

    async fn shutdown_session(&self, session: &Self::Session) {
        let _ = session.shutdown_and_wait().await;
    }

    fn session_id(&self, session: &Self::Session) -> String {
        session.session.conversation_id.to_string()
    }

    async fn trunk_rollout_path(&self, session: &Self::Session) -> Option<PathBuf> {
        session.session.ensure_rollout_materialized().await;
        match session.session.current_rollout_path().await {
            Ok(path) => path,
            Err(err) => {
                warn!("failed to resolve guardian trunk rollout path: {err}");
                None
            }
        }
    }

    async fn load_rollout_items_for_fork(
        &self,
        session: &Self::Session,
    ) -> anyhow::Result<Option<Vec<RolloutItem>>> {
        session.session.try_ensure_rollout_materialized().await?;
        session.session.flush_rollout().await?;
        let live_thread = session
            .session
            .live_thread_for_persistence("guardian review fork")?;
        let history = live_thread.load_history(/*include_archived*/ true).await?;
        Ok(Some(history.items))
    }

    async fn append_followup_reminder(&self, session: &Self::Session) {
        let turn_context = session.session.new_default_turn().await;
        let reminder: ResponseItem = ContextualUserFragment::into(GuardianFollowupReviewReminder);
        session
            .session
            .record_conversation_items(turn_context.as_ref(), std::slice::from_ref(&reminder))
            .await;
    }

    async fn model_info(&self, request: &Self::Request) -> GuardianReviewModelInfo {
        let model_info = self
            .services
            .models_manager
            .get_model_info(
                request.model.as_str(),
                &request.spawn_config.to_models_manager_config(),
            )
            .await;
        GuardianReviewModelInfo {
            supports_reasoning_summaries: model_info.supports_reasoning_summaries,
            default_reasoning_level: model_info.default_reasoning_level,
        }
    }

    async fn sync_network_approval(&self, _request: &Self::Request, session: &Self::Session) {
        self.services
            .network_approval
            .sync_session_approved_hosts_to(&session.session.services.network_approval)
            .await;
    }

    async fn build_prompt_items(
        &self,
        _request: &Self::Request,
        retry_reason: Option<String>,
        approval_request: GuardianApprovalRequest,
        prompt_mode: GuardianPromptMode,
    ) -> serde_json::Result<GuardianPromptItems> {
        let history = self.clone_history().await;
        let transcript_entries = collect_guardian_transcript_entries(history.raw_items());
        build_guardian_prompt_items_from_entries(
            &self.conversation_id.to_string(),
            history.history_version(),
            transcript_entries.as_slice(),
            retry_reason,
            approval_request,
            prompt_mode,
        )
    }

    async fn total_token_usage(&self, session: &Self::Session) -> Option<TokenUsage> {
        session.session.total_token_usage().await
    }

    async fn submit_review(
        &self,
        session: &Self::Session,
        request: &Self::Request,
        items: Vec<UserInput>,
        schema: Value,
        model: String,
        reasoning_effort: Option<ReasoningEffortConfig>,
        reasoning_summary: ReasoningSummaryConfig,
        personality: Option<Personality>,
    ) -> anyhow::Result<String> {
        let child_turn_id = session
            .submit(Op::UserTurn {
                environments: None,
                items,
                #[allow(deprecated)]
                cwd: request.parent_turn.cwd.to_path_buf(),
                approval_policy: AskForApproval::Never,
                approvals_reviewer: None,
                sandbox_policy: SandboxPolicy::new_read_only_policy(),
                permission_profile: None,
                model,
                effort: reasoning_effort,
                summary: Some(reasoning_summary),
                service_tier: None,
                final_output_json_schema: Some(schema),
                collaboration_mode: None,
                personality,
            })
            .await?;
        Ok(child_turn_id)
    }

    async fn next_event(&self, session: &Self::Session) -> anyhow::Result<Event> {
        Ok(session.next_event().await?)
    }

    async fn interrupt_and_drain(
        &self,
        session: &Self::Session,
        expected_turn_id: &str,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        let _ = session.submit(Op::Interrupt).await;

        tokio::time::timeout(timeout, async {
            loop {
                let event = session.next_event().await?;
                if event_matches_turn(&event, expected_turn_id)
                    && matches!(
                        event.msg,
                        codex_protocol::protocol::EventMsg::TurnAborted(_)
                            | codex_protocol::protocol::EventMsg::TurnComplete(_)
                    )
                {
                    return Ok::<(), anyhow::Error>(());
                }
            }
        })
        .await
        .map_err(|_| anyhow!("timed out draining guardian review session after interrupt"))??;

        Ok(())
    }

    #[doc(hidden)]
    async fn send_event_raw_for_test(&self, session: &Self::Session, event: Event) {
        session.session.send_event_raw(event).await;
    }
}

pub(crate) fn build_guardian_prompt_items_from_session_history(
    session: &Session,
    retry_reason: Option<String>,
    request: GuardianApprovalRequest,
    mode: GuardianPromptMode,
) -> impl std::future::Future<Output = serde_json::Result<GuardianPromptItems>> + '_ {
    async move {
        let history = session.clone_history().await;
        let transcript_entries = collect_guardian_transcript_entries(history.raw_items());
        build_guardian_prompt_items_from_entries(
            &session.conversation_id.to_string(),
            history.history_version(),
            transcript_entries.as_slice(),
            retry_reason,
            request,
            mode,
        )
    }
}

pub(crate) fn build_guardian_review_session_config(
    parent_config: &Config,
    live_network_config: Option<NetworkProxyConfig>,
    active_model: &str,
    reasoning_effort: Option<codex_protocol::openai_models::ReasoningEffort>,
) -> anyhow::Result<Config> {
    let mut guardian_config = parent_config.clone();
    guardian_config.model = Some(active_model.to_string());
    guardian_config.model_reasoning_effort = reasoning_effort;
    guardian_config.include_skill_instructions = false;
    guardian_config.base_instructions = Some(
        parent_config
            .guardian_policy_config
            .as_deref()
            .map(guardian_policy_prompt_with_config)
            .unwrap_or_else(guardian_policy_prompt),
    );
    guardian_config.developer_instructions = None;
    guardian_config.permissions.approval_policy = Constrained::allow_only(AskForApproval::Never);
    let sandbox_policy = SandboxPolicy::new_read_only_policy();
    guardian_config
        .permissions
        .set_legacy_sandbox_policy(sandbox_policy, guardian_config.cwd.as_path())
        .map_err(|err| {
            anyhow::anyhow!("guardian review session could not set sandbox policy: {err}")
        })?;
    guardian_config.include_apps_instructions = false;
    guardian_config
        .mcp_servers
        .set(HashMap::new())
        .map_err(|err| {
            anyhow::anyhow!("guardian review session could not clear MCP servers: {err}")
        })?;
    if let Some(live_network_config) = live_network_config
        && guardian_config.permissions.network.is_some()
    {
        let network_constraints = guardian_config
            .config_layer_stack
            .requirements()
            .network
            .as_ref()
            .map(|network| network.value.clone());
        guardian_config.permissions.network = Some(NetworkProxySpec::from_config_and_constraints(
            live_network_config,
            network_constraints,
            guardian_config.permissions.permission_profile(),
        )?);
    }
    for feature in [
        Feature::SpawnCsv,
        Feature::Collab,
        Feature::MultiAgentV2,
        Feature::CodexHooks,
        Feature::Apps,
        Feature::Plugins,
        Feature::WebSearchRequest,
        Feature::WebSearchCached,
    ] {
        guardian_config.features.disable(feature).map_err(|err| {
            anyhow::anyhow!(
                "guardian review session could not disable `features.{}`: {err}",
                feature.key()
            )
        })?;
        if guardian_config.features.enabled(feature) {
            warn!(
                "guardian review session could not disable `features.{}`; continuing with the feature enabled",
                feature.key()
            );
        }
    }
    Ok(guardian_config)
}

fn event_matches_turn(event: &Event, expected_turn_id: &str) -> bool {
    if event.id != expected_turn_id {
        return false;
    }

    match &event.msg {
        codex_protocol::protocol::EventMsg::TurnComplete(turn_complete) => {
            turn_complete.turn_id == expected_turn_id
        }
        codex_protocol::protocol::EventMsg::TurnAborted(turn_aborted) => {
            turn_aborted.turn_id.as_deref() == Some(expected_turn_id)
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn guardian_review_session_config_change_invalidates_cached_session() {
        let parent_config = crate::config::test_config().await;
        let cached_spawn_config = build_guardian_review_session_config(
            &parent_config,
            /*live_network_config*/ None,
            "active-model",
            /*reasoning_effort*/ None,
        )
        .expect("cached guardian config");
        let cached_reuse_key =
            GuardianReviewSessionReuseKey::from_spawn_config(&cached_spawn_config);

        let mut changed_parent_config = parent_config;
        changed_parent_config.model_provider.base_url =
            Some("https://guardian.example.invalid/v1".to_string());
        let next_spawn_config = build_guardian_review_session_config(
            &changed_parent_config,
            /*live_network_config*/ None,
            "active-model",
            /*reasoning_effort*/ None,
        )
        .expect("next guardian config");
        let next_reuse_key = GuardianReviewSessionReuseKey::from_spawn_config(&next_spawn_config);

        assert_ne!(cached_reuse_key, next_reuse_key);
        assert_eq!(
            cached_reuse_key,
            GuardianReviewSessionReuseKey::from_spawn_config(&cached_spawn_config)
        );
    }

    #[tokio::test]
    async fn guardian_review_session_config_disables_hooks() {
        let mut parent_config = crate::config::test_config().await;
        parent_config
            .features
            .enable(Feature::CodexHooks)
            .expect("enable hooks on parent config");

        let guardian_config = build_guardian_review_session_config(
            &parent_config,
            /*live_network_config*/ None,
            "active-model",
            /*reasoning_effort*/ None,
        )
        .expect("guardian config");

        assert!(!guardian_config.features.enabled(Feature::CodexHooks));
    }

    #[tokio::test]
    async fn guardian_review_session_config_disables_skill_instructions() {
        let mut parent_config = crate::config::test_config().await;
        parent_config.include_skill_instructions = true;

        let guardian_config = build_guardian_review_session_config(
            &parent_config,
            /*live_network_config*/ None,
            "active-model",
            /*reasoning_effort*/ None,
        )
        .expect("guardian config");

        assert!(!guardian_config.include_skill_instructions);
    }

    #[test]
    fn event_match_uses_turn_specific_terminal_events() {
        let turn_id = "turn-1";
        let event = Event {
            id: turn_id.to_string(),
            msg: codex_protocol::protocol::EventMsg::TurnComplete(
                codex_protocol::protocol::TurnCompleteEvent {
                    turn_id: turn_id.to_string(),
                    last_agent_message: None,
                    completed_at: None,
                    duration_ms: None,
                    time_to_first_token_ms: None,
                },
            ),
        };

        assert!(event_matches_turn(&event, turn_id));
        assert!(!event_matches_turn(&event, "other-turn"));
    }
}
