use crate::agent::AgentStatus;
use crate::goal::GoalRuntimeEvent;
use crate::session::Codex;
use crate::session::SessionSettingsUpdate;
use crate::session::SteerInputError;
use codex_agent_runtime::ThreadIdleReason;
use codex_agent_runtime::ThreadPostTurnState;
use codex_config::ConstraintResult;
use codex_features::Feature;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::mcp::CallToolResult;
use codex_protocol::mcp::ReadResourceRequestParams;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::protocol::SessionConfiguredEvent;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::Submission;
use codex_protocol::protocol::ThreadContextUsage;
use codex_protocol::protocol::ThreadMemoryMode;
use codex_protocol::protocol::TokenUsageInfo;
use codex_protocol::protocol::TurnEnvironmentSelection;
use codex_protocol::protocol::W3cTraceContext;
use codex_protocol::user_input::UserInput;
use crate::PendingInputItem;
use codex_session_telemetry_api::SharedSessionTelemetry;
use codex_state_api::ExternalGoalSet;
use codex_thread_api::AppServerClientInfo;
use codex_thread_api::CodexThreadTurnContextOverrides;
use codex_thread_api::ThreadConfigSnapshot;
use codex_thread_api::ThreadRuntimeStatus;
use codex_thread_store_api::StoredThread;
use codex_thread_store_api::StoredThreadHistory;
use codex_thread_store_api::ThreadMetadataPatch;
use codex_thread_store_api::ThreadStoreError;
use codex_thread_store_api::ThreadStoreResult;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::watch;

use crate::StateDbHandle;

pub(crate) fn thread_config_snapshot_sandbox_policy(
    config_snapshot: &ThreadConfigSnapshot,
) -> SandboxPolicy {
    let file_system_sandbox_policy = config_snapshot
        .permission_profile
        .file_system_sandbox_policy();
    codex_sandboxing_api::compatibility_sandbox_policy_for_permission_profile(
        &config_snapshot.permission_profile,
        &file_system_sandbox_policy,
        config_snapshot.permission_profile.network_sandbox_policy(),
        config_snapshot.cwd.as_path(),
    )
}

pub struct CodexThread {
    pub(crate) codex: Codex,
    pub(crate) session_source: SessionSource,
    session_configured: SessionConfiguredEvent,
    rollout_path: Option<PathBuf>,
    out_of_band_elicitation_count: Mutex<u64>,
}

/// Conduit for the bidirectional stream of messages that compose a thread
/// (formerly called a conversation) in Codex.
impl CodexThread {
    pub(crate) fn new(
        codex: Codex,
        session_configured: SessionConfiguredEvent,
        rollout_path: Option<PathBuf>,
        session_source: SessionSource,
    ) -> Self {
        Self {
            codex,
            session_source,
            session_configured,
            rollout_path,
            out_of_band_elicitation_count: Mutex::new(0),
        }
    }

    pub async fn runtime_thread_status(&self) -> ThreadRuntimeStatus {
        match self.codex.session.thread_post_turn_state().await {
            ThreadPostTurnState::ThreadActive
            | ThreadPostTurnState::GoContextContinuation { .. } => ThreadRuntimeStatus::Active,
            ThreadPostTurnState::ThreadIdle(ThreadIdleReason::WaitCommand) => {
                ThreadRuntimeStatus::IdleWaitCommand
            }
            ThreadPostTurnState::ThreadIdle(ThreadIdleReason::WaitChild) => {
                ThreadRuntimeStatus::IdleWaitChild
            }
            ThreadPostTurnState::ThreadCompletion => ThreadRuntimeStatus::Complete,
        }
    }

    pub async fn submit(&self, op: Op) -> CodexResult<String> {
        self.codex.submit(op).await
    }

    /// Returns the session telemetry handle for thread-scoped production instrumentation.
    pub fn session_telemetry(&self) -> SharedSessionTelemetry {
        self.codex.session.services.session_telemetry.clone()
    }

    pub async fn shutdown_and_wait(&self) -> CodexResult<()> {
        self.codex.shutdown_and_wait().await
    }

    /// Wait until the underlying session loop has terminated.
    pub async fn wait_until_terminated(&self) {
        self.codex.session_loop_termination.clone().await;
    }

    pub(crate) fn emit_thread_resume_lifecycle(&self) {
        for contributor in self
            .codex
            .session
            .services
            .extensions
            .thread_lifecycle_contributors()
        {
            contributor.on_thread_resume(codex_extension_api::ThreadResumeInput {
                session_store: &self.codex.session.services.session_extension_data,
                thread_store: &self.codex.session.services.thread_extension_data,
            });
        }
    }

    pub async fn apply_goal_resume_runtime_effects(&self) -> anyhow::Result<()> {
        self.codex
            .session
            .goal_runtime_apply(GoalRuntimeEvent::ThreadResumed)
            .await
    }

    pub async fn continue_active_goal_if_idle(&self) -> anyhow::Result<()> {
        self.codex
            .session
            .goal_runtime_apply(GoalRuntimeEvent::MaybeContinueIfIdle)
            .await
    }

    pub async fn prepare_external_goal_mutation(&self) {
        if let Err(err) = self
            .codex
            .session
            .goal_runtime_apply(GoalRuntimeEvent::ExternalMutationStarting)
            .await
        {
            tracing::warn!("failed to prepare external goal mutation: {err}");
        }
    }

    pub async fn apply_external_goal_set(&self, external_set: ExternalGoalSet) {
        if let Err(err) = self
            .codex
            .session
            .goal_runtime_apply(GoalRuntimeEvent::ExternalSet { external_set })
            .await
        {
            tracing::warn!("failed to apply external goal status runtime effects: {err}");
        }
    }

    pub async fn apply_external_goal_clear(&self) {
        if let Err(err) = self
            .codex
            .session
            .goal_runtime_apply(GoalRuntimeEvent::ExternalClear)
            .await
        {
            tracing::warn!("failed to apply external goal clear runtime effects: {err}");
        }
    }

    #[doc(hidden)]
    pub async fn ensure_rollout_materialized(&self) {
        self.codex.session.ensure_rollout_materialized().await;
    }

    #[doc(hidden)]
    pub async fn flush_rollout(&self) -> std::io::Result<()> {
        self.codex.session.flush_rollout().await
    }

    pub async fn submit_with_trace(
        &self,
        op: Op,
        trace: Option<W3cTraceContext>,
    ) -> CodexResult<String> {
        self.codex.submit_with_trace(op, trace).await
    }

    /// Persist whether this thread is eligible for future memory generation.
    pub async fn set_thread_memory_mode(&self, mode: ThreadMemoryMode) -> anyhow::Result<()> {
        self.codex.set_thread_memory_mode(mode).await
    }

    pub async fn steer_input(
        &self,
        input: Vec<UserInput>,
        expected_turn_id: Option<&str>,
        responsesapi_client_metadata: Option<HashMap<String, String>>,
    ) -> Result<String, SteerInputError> {
        self.codex
            .steer_input(input, expected_turn_id, responsesapi_client_metadata)
            .await
    }

    pub async fn set_app_server_client_info(
        &self,
        app_server_client_name: Option<String>,
        app_server_client_version: Option<String>,
        mcp_elicitations_auto_deny: bool,
    ) -> ConstraintResult<()> {
        self.codex
            .set_app_server_client_info(
                app_server_client_name,
                app_server_client_version,
                mcp_elicitations_auto_deny,
            )
            .await
    }

    /// Validate persistent turn context overrides without committing them.
    pub async fn validate_turn_context_overrides(
        &self,
        overrides: CodexThreadTurnContextOverrides,
    ) -> ConstraintResult<()> {
        let CodexThreadTurnContextOverrides {
            cwd,
            workspace_roots,
            profile_workspace_roots,
            approval_policy,
            approvals_reviewer,
            sandbox_policy,
            permission_profile,
            active_permission_profile,
            windows_sandbox_level,
            model_provider,
            model,
            effort,
            summary,
            service_tier,
            collaboration_mode,
            personality,
        } = overrides;
        let collaboration_mode = if let Some(collaboration_mode) = collaboration_mode {
            collaboration_mode
        } else {
            self.codex
                .session
                .collaboration_mode()
                .await
                .with_updates(model, effort, /*developer_instructions*/ None)
        };

        let updates = SessionSettingsUpdate {
            cwd,
            workspace_roots,
            profile_workspace_roots,
            approval_policy,
            approvals_reviewer,
            sandbox_policy,
            permission_profile,
            active_permission_profile,
            windows_sandbox_level,
            model_provider,
            collaboration_mode: Some(collaboration_mode),
            reasoning_summary: summary,
            service_tier,
            personality,
            ..Default::default()
        };
        self.codex.session.validate_settings(&updates).await
    }

    /// Use sparingly: this is intended to be removed soon.
    pub async fn submit_with_id(&self, sub: Submission) -> CodexResult<()> {
        self.codex.submit_with_id(sub).await
    }

    pub async fn next_event(&self) -> CodexResult<Event> {
        self.codex.next_event().await
    }

    pub async fn agent_status(&self) -> AgentStatus {
        self.codex.agent_status().await
    }

    pub(crate) fn subscribe_status(&self) -> watch::Receiver<AgentStatus> {
        self.codex.agent_status.clone()
    }

    /// Returns the complete token usage snapshot currently cached for this thread.
    ///
    /// This accessor is intentionally narrower than direct session access: it lets
    /// app-server lifecycle paths replay restored usage after resume or fork without
    /// exposing broader session mutation authority. A caller that only reads
    /// `total_token_usage` would drop last-turn usage and make the v2
    /// `thread/tokenUsage/updated` payload incomplete.
    pub async fn token_usage_info(&self) -> Option<TokenUsageInfo> {
        self.codex.session.token_usage_info().await
    }

    /// Returns a context usage snapshot computed from the thread's live history.
    pub async fn thread_context_usage(&self) -> ThreadContextUsage {
        self.codex.session.thread_context_usage().await
    }

    /// Records a user-role session-prefix message without creating a new user turn boundary.
    pub(crate) async fn inject_user_message_without_turn(&self, message: String) {
        let content = vec![ContentItem::InputText { text: message }];
        let message = ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: content.clone(),
            phase: None,
        };
        let pending_item = ResponseInputItem::Message {
            role: "user".to_string(),
            content,
            phase: None,
        };
        if self
            .codex
            .session
            .inject_hook_inspectable_items(vec![pending_item])
            .await
            .is_err()
        {
            let turn_context = self.codex.session.new_default_turn().await;
            self.codex
                .session
                .record_conversation_items(turn_context.as_ref(), &[message])
                .await;
        }
    }

    /// Append a prebuilt message to the thread history without treating it as a user turn.
    ///
    /// If the thread already has an active turn, the message is queued as pending input for that
    /// turn. Otherwise it is queued at session scope and a regular turn is started so the agent
    /// can consume that pending input through the normal turn pipeline.
    pub async fn append_message(&self, message: ResponseItem) -> CodexResult<String> {
        let submission_id = uuid::Uuid::new_v4().to_string();
        self.codex
            .session
            .enqueue_async_input(PendingInputItem::from(message));
        self.codex.session.maybe_start_turn_for_pending_work().await;

        Ok(submission_id)
    }

    /// Append raw Responses API items to the thread's model-visible history.
    pub async fn inject_conversation_items(&self, items: Vec<ResponseItem>) -> CodexResult<()> {
        if items.is_empty() {
            return Err(CodexErr::InvalidRequest(
                "items must not be empty".to_string(),
            ));
        }

        let turn_context = self.codex.session.new_default_turn().await;
        if self.codex.session.reference_context_item().await.is_none() {
            self.codex
                .session
                .record_context_updates_and_set_reference_context_item(turn_context.as_ref())
                .await;
        }
        self.codex
            .session
            .record_conversation_items(turn_context.as_ref(), &items)
            .await;
        self.codex.session.flush_rollout().await?;
        Ok(())
    }

    pub fn rollout_path(&self) -> Option<PathBuf> {
        self.rollout_path.clone()
    }

    pub fn session_configured(&self) -> SessionConfiguredEvent {
        self.session_configured.clone()
    }

    pub(crate) fn is_running(&self) -> bool {
        !self.codex.tx_sub.is_closed()
    }

    pub async fn guardian_trunk_rollout_path(&self) -> Option<PathBuf> {
        self.codex
            .session
            .guardian_review_session
            .trunk_rollout_path()
            .await
    }

    pub async fn load_history(
        &self,
        include_archived: bool,
    ) -> ThreadStoreResult<StoredThreadHistory> {
        let live_thread = self
            .codex
            .session
            .live_thread_for_persistence("load history")
            .map_err(|err| ThreadStoreError::Internal {
                message: err.to_string(),
            })?;
        live_thread.load_history(include_archived).await
    }

    pub async fn read_thread(
        &self,
        include_archived: bool,
        include_history: bool,
    ) -> ThreadStoreResult<StoredThread> {
        let live_thread = self
            .codex
            .session
            .live_thread_for_persistence("read thread")
            .map_err(|err| ThreadStoreError::Internal {
                message: err.to_string(),
            })?;
        live_thread
            .read_thread(include_archived, include_history)
            .await
    }

    pub async fn update_thread_metadata(
        &self,
        patch: ThreadMetadataPatch,
        include_archived: bool,
    ) -> ThreadStoreResult<StoredThread> {
        let live_thread = self
            .codex
            .session
            .live_thread_for_persistence("update thread metadata")
            .map_err(|err| ThreadStoreError::Internal {
                message: err.to_string(),
            })?;
        live_thread.update_metadata(patch, include_archived).await
    }

    pub fn state_db(&self) -> Option<StateDbHandle> {
        self.codex.state_db()
    }

    pub async fn config_snapshot(&self) -> ThreadConfigSnapshot {
        self.codex.thread_config_snapshot().await
    }

    pub async fn config(&self) -> Arc<codex_config::Config> {
        self.codex.session.get_config().await
    }

    /// Refresh the thread's layer-backed user config state from a caller-supplied
    /// config snapshot. Thread-scoped layers and session-static settings remain
    /// unchanged.
    pub async fn refresh_runtime_config(&self, next_config: codex_config::Config) {
        self.codex.session.refresh_runtime_config(next_config).await;
    }

    pub async fn environment_selections(&self) -> Vec<TurnEnvironmentSelection> {
        self.codex.thread_environment_selections().await
    }

    pub async fn read_mcp_resource(
        &self,
        server: &str,
        uri: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let result = self
            .codex
            .session
            .read_resource(
                server,
                ReadResourceRequestParams {
                    uri: uri.to_string(),
                },
            )
            .await?;

        Ok(serde_json::to_value(result)?)
    }

    pub async fn call_mcp_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Option<serde_json::Value>,
        meta: Option<serde_json::Value>,
    ) -> anyhow::Result<CallToolResult> {
        self.codex
            .session
            .call_tool(server, tool, arguments, meta)
            .await
    }

    pub fn enabled(&self, feature: Feature) -> bool {
        self.codex.enabled(feature)
    }

    pub async fn increment_out_of_band_elicitation_count(&self) -> CodexResult<u64> {
        let mut guard = self.out_of_band_elicitation_count.lock().await;
        let was_zero = *guard == 0;
        *guard = guard.checked_add(1).ok_or_else(|| {
            CodexErr::Fatal("out-of-band elicitation count overflowed".to_string())
        })?;

        if was_zero {
            self.codex
                .session
                .set_out_of_band_elicitation_pause_state(/*paused*/ true);
        }

        Ok(*guard)
    }

    pub async fn decrement_out_of_band_elicitation_count(&self) -> CodexResult<u64> {
        let mut guard = self.out_of_band_elicitation_count.lock().await;
        if *guard == 0 {
            return Err(CodexErr::InvalidRequest(
                "out-of-band elicitation count is already zero".to_string(),
            ));
        }

        *guard -= 1;
        let now_zero = *guard == 0;
        if now_zero {
            self.codex
                .session
                .set_out_of_band_elicitation_pause_state(/*paused*/ false);
        }

        Ok(*guard)
    }
}

impl codex_thread_api::SessionCommandHandle for CodexThread {
    fn submit_op(
        &self,
        op: Op,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        self.submit(op)
    }

    fn submit_op_with_trace(
        &self,
        op: Op,
        trace: Option<W3cTraceContext>,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        self.submit_with_trace(op, trace)
    }

    fn submit_with_id(
        &self,
        submission: Submission,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        CodexThread::submit_with_id(self, submission)
    }

    fn shutdown(&self) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        self.shutdown_and_wait()
    }

    fn append_conversation_item(
        &self,
        item: ResponseItem,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        self.append_message(item)
    }
}

impl codex_thread_api::LiveThreadHandle for CodexThread {
    fn session_configured(&self) -> SessionConfiguredEvent {
        CodexThread::session_configured(self)
    }

    fn next_event(&self) -> impl std::future::Future<Output = CodexResult<Event>> + Send + '_ {
        CodexThread::next_event(self)
    }

    fn submit_thread_op(
        &self,
        op: Op,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        self.submit(op)
    }

    fn agent_status(&self) -> impl std::future::Future<Output = AgentStatus> + Send + '_ {
        CodexThread::agent_status(self)
    }

    fn runtime_thread_status(
        &self,
    ) -> impl std::future::Future<Output = ThreadRuntimeStatus> + Send + '_ {
        CodexThread::runtime_thread_status(self)
    }

    fn feature_enabled(&self, feature: Feature) -> bool {
        CodexThread::enabled(self, feature)
    }

    fn config_snapshot(
        &self,
    ) -> impl std::future::Future<Output = ThreadConfigSnapshot> + Send + '_ {
        CodexThread::config_snapshot(self)
    }

    fn guardian_trunk_rollout_path(
        &self,
    ) -> impl std::future::Future<Output = Option<PathBuf>> + Send + '_ {
        CodexThread::guardian_trunk_rollout_path(self)
    }

    fn set_app_server_client_info(
        &self,
        info: AppServerClientInfo,
    ) -> impl std::future::Future<Output = ConstraintResult<()>> + Send + '_ {
        CodexThread::set_app_server_client_info(
            self,
            info.app_server_client_name,
            info.app_server_client_version,
            info.mcp_elicitations_auto_deny,
        )
    }

    fn validate_turn_context_overrides(
        &self,
        overrides: CodexThreadTurnContextOverrides,
    ) -> impl std::future::Future<Output = ConstraintResult<()>> + Send + '_ {
        CodexThread::validate_turn_context_overrides(self, overrides)
    }

    fn token_usage_info(
        &self,
    ) -> impl std::future::Future<Output = Option<TokenUsageInfo>> + Send + '_ {
        CodexThread::token_usage_info(self)
    }

    fn thread_context_usage(
        &self,
    ) -> impl std::future::Future<Output = ThreadContextUsage> + Send + '_ {
        CodexThread::thread_context_usage(self)
    }

    fn load_history(
        &self,
        include_archived: bool,
    ) -> impl std::future::Future<Output = ThreadStoreResult<StoredThreadHistory>> + Send + '_ {
        CodexThread::load_history(self, include_archived)
    }

    fn read_thread(
        &self,
        include_archived: bool,
        include_history: bool,
    ) -> impl std::future::Future<Output = ThreadStoreResult<StoredThread>> + Send + '_ {
        CodexThread::read_thread(self, include_archived, include_history)
    }

    fn shutdown_and_wait(&self) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        CodexThread::shutdown_and_wait(self)
    }

    fn wait_until_terminated(&self) -> impl std::future::Future<Output = ()> + Send + '_ {
        CodexThread::wait_until_terminated(self)
    }

    fn prepare_external_goal_mutation(&self) -> impl std::future::Future<Output = ()> + Send + '_ {
        CodexThread::prepare_external_goal_mutation(self)
    }

    fn apply_goal_resume_runtime_effects(
        &self,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        async move {
            CodexThread::apply_goal_resume_runtime_effects(self)
                .await
                .map_err(|err| CodexErr::Fatal(err.to_string()))
        }
    }

    fn continue_active_goal_if_idle(
        &self,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        async move {
            CodexThread::continue_active_goal_if_idle(self)
                .await
                .map_err(|err| CodexErr::Fatal(err.to_string()))
        }
    }

    fn apply_external_goal_set(
        &self,
        external_set: ExternalGoalSet,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        CodexThread::apply_external_goal_set(self, external_set)
    }

    fn apply_external_goal_clear(&self) -> impl std::future::Future<Output = ()> + Send + '_ {
        CodexThread::apply_external_goal_clear(self)
    }

    fn increment_out_of_band_elicitation_count(
        &self,
    ) -> impl std::future::Future<Output = CodexResult<u64>> + Send + '_ {
        CodexThread::increment_out_of_band_elicitation_count(self)
    }

    fn decrement_out_of_band_elicitation_count(
        &self,
    ) -> impl std::future::Future<Output = CodexResult<u64>> + Send + '_ {
        CodexThread::decrement_out_of_band_elicitation_count(self)
    }
}
