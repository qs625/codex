use std::fmt::Debug;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use codex_analytics_api::GuardianReviewAnalyticsResult;
use codex_analytics_api::GuardianReviewSessionKind;
use protocol::config_types::Personality;
use protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use protocol::protocol::Event;
use protocol::protocol::EventMsg;
use protocol::protocol::InitialHistory;
use protocol::protocol::RolloutItem;
use protocol::protocol::TokenUsage;
use protocol::user_input::UserInput;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::GuardianApprovalRequest;
use crate::GuardianPromptItems;
use crate::GuardianPromptMode;
use crate::GuardianTranscriptCursor;

pub const GUARDIAN_REVIEW_TIMEOUT: Duration = Duration::from_secs(90);
const GUARDIAN_INTERRUPT_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardianReviewSpawnKind {
    Trunk,
    Ephemeral,
}

#[derive(Clone, Debug)]
pub struct GuardianReviewForkSnapshot {
    pub initial_history: InitialHistory,
    pub prior_review_count: usize,
    pub last_reviewed_transcript_cursor: Option<GuardianTranscriptCursor>,
}

#[derive(Clone, Debug)]
pub struct GuardianReviewModelInfo {
    pub supports_reasoning_summaries: bool,
    pub default_reasoning_level: Option<ReasoningEffortConfig>,
}

#[derive(Debug)]
pub enum GuardianReviewSessionOutcome {
    Completed(anyhow::Result<Option<String>>),
    PromptBuildFailed(anyhow::Error),
    SessionFailed(anyhow::Error),
    TimedOut,
    Aborted,
}

/// Host boundary for guardian review session orchestration.
///
/// Implementations own concrete thread/session capabilities such as spawning
/// the reviewer, submitting a turn, reading events, and loading rollout state.
/// The guardian crate owns reuse, timeout, fork, and analytics state around
/// those capabilities without depending on the concrete session runtime.
pub trait GuardianReviewSessionHost: Send + Sync + 'static {
    type Request: Send + Sync;
    type ReuseKey: Clone + Debug + PartialEq + Send + Sync + 'static;
    type Session: Send + Sync + 'static;

    fn spawn_session<'a>(
        &'a self,
        request: &'a Self::Request,
        spawn_kind: GuardianReviewSpawnKind,
        cancel_token: CancellationToken,
        fork_snapshot: Option<GuardianReviewForkSnapshot>,
    ) -> impl Future<Output = anyhow::Result<Self::Session>> + Send + 'a;

    fn shutdown_session<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn session_id(&self, session: &Self::Session) -> String;

    fn trunk_rollout_path<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl Future<Output = Option<PathBuf>> + Send + 'a;

    fn load_rollout_items_for_fork<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl Future<Output = anyhow::Result<Option<Vec<RolloutItem>>>> + Send + 'a;

    fn append_followup_reminder<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn model_info<'a>(
        &'a self,
        request: &'a Self::Request,
    ) -> impl Future<Output = GuardianReviewModelInfo> + Send + 'a;

    fn sync_network_approval<'a>(
        &'a self,
        request: &'a Self::Request,
        session: &'a Self::Session,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn build_prompt_items<'a>(
        &'a self,
        request: &'a Self::Request,
        retry_reason: Option<String>,
        approval_request: GuardianApprovalRequest,
        prompt_mode: GuardianPromptMode,
    ) -> impl Future<Output = serde_json::Result<GuardianPromptItems>> + Send + 'a;

    fn total_token_usage<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl Future<Output = Option<TokenUsage>> + Send + 'a;

    #[allow(clippy::too_many_arguments)]
    fn submit_review<'a>(
        &'a self,
        session: &'a Self::Session,
        request: &'a Self::Request,
        items: Vec<UserInput>,
        schema: Value,
        model: String,
        reasoning_effort: Option<ReasoningEffortConfig>,
        reasoning_summary: ReasoningSummaryConfig,
        personality: Option<Personality>,
    ) -> impl Future<Output = anyhow::Result<String>> + Send + 'a;

    fn next_event<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl Future<Output = anyhow::Result<Event>> + Send + 'a;

    fn interrupt_and_drain<'a>(
        &'a self,
        session: &'a Self::Session,
        expected_turn_id: &'a str,
        timeout: Duration,
    ) -> impl Future<Output = anyhow::Result<()>> + Send + 'a;

    #[doc(hidden)]
    fn send_event_raw_for_test<'a>(
        &'a self,
        _session: &'a Self::Session,
        _event: Event,
    ) -> impl Future<Output = ()> + Send + 'a {
        async {}
    }
}

pub struct GuardianReviewSessionParams<H>
where
    H: GuardianReviewSessionHost,
{
    pub host: Arc<H>,
    pub host_request: H::Request,
    pub reuse_key: H::ReuseKey,
    pub request: GuardianApprovalRequest,
    pub retry_reason: Option<String>,
    pub schema: Value,
    pub model: String,
    pub reasoning_effort: Option<ReasoningEffortConfig>,
    pub reasoning_summary: ReasoningSummaryConfig,
    pub personality: Option<Personality>,
    pub external_cancel: Option<CancellationToken>,
}

pub struct GuardianReviewSessionManager<H>
where
    H: GuardianReviewSessionHost,
{
    state: Arc<Mutex<GuardianReviewSessionState<H>>>,
}

struct GuardianReviewSessionState<H>
where
    H: GuardianReviewSessionHost,
{
    trunk: Option<Arc<GuardianReviewSession<H>>>,
    ephemeral_reviews: Vec<Arc<GuardianReviewSession<H>>>,
}

struct GuardianReviewSession<H>
where
    H: GuardianReviewSessionHost,
{
    host: Arc<H>,
    session: H::Session,
    cancel_token: CancellationToken,
    reuse_key: H::ReuseKey,
    review_lock: Semaphore,
    state: Mutex<GuardianReviewState>,
}

struct GuardianReviewState {
    prior_review_count: usize,
    last_reviewed_transcript_cursor: Option<GuardianTranscriptCursor>,
    last_committed_fork_snapshot: Option<GuardianReviewForkSnapshot>,
}

struct EphemeralReviewCleanup<H>
where
    H: GuardianReviewSessionHost,
{
    state: Arc<Mutex<GuardianReviewSessionState<H>>>,
    review_session: Option<Arc<GuardianReviewSession<H>>>,
}

impl<H> Default for GuardianReviewSessionManager<H>
where
    H: GuardianReviewSessionHost,
{
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(GuardianReviewSessionState {
                trunk: None,
                ephemeral_reviews: Vec::new(),
            })),
        }
    }
}

fn had_prior_review_context(prompt_mode: &GuardianPromptMode) -> bool {
    matches!(prompt_mode, GuardianPromptMode::Delta { .. })
}

fn token_usage_delta(start: &TokenUsage, end: &TokenUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: (end.input_tokens - start.input_tokens).max(0),
        cached_input_tokens: (end.cached_input_tokens - start.cached_input_tokens).max(0),
        output_tokens: (end.output_tokens - start.output_tokens).max(0),
        reasoning_output_tokens: (end.reasoning_output_tokens - start.reasoning_output_tokens)
            .max(0),
        total_tokens: (end.total_tokens - start.total_tokens).max(0),
    }
}

impl<H> GuardianReviewSession<H>
where
    H: GuardianReviewSessionHost,
{
    async fn shutdown(&self) {
        self.cancel_token.cancel();
        self.host.shutdown_session(&self.session).await;
    }

    fn shutdown_in_background(self: &Arc<Self>) {
        let review_session = Arc::clone(self);
        drop(tokio::spawn(async move {
            review_session.shutdown().await;
        }));
    }

    async fn fork_snapshot(&self) -> Option<GuardianReviewForkSnapshot> {
        self.state.lock().await.last_committed_fork_snapshot.clone()
    }

    async fn refresh_last_committed_fork_snapshot(&self) {
        match self.host.load_rollout_items_for_fork(&self.session).await {
            Ok(Some(items)) if !items.is_empty() => {
                let mut state = self.state.lock().await;
                let prior_review_count = state.prior_review_count;
                let last_reviewed_transcript_cursor = state.last_reviewed_transcript_cursor;
                state.last_committed_fork_snapshot = Some(GuardianReviewForkSnapshot {
                    initial_history: InitialHistory::Forked(items),
                    prior_review_count,
                    last_reviewed_transcript_cursor,
                });
            }
            Ok(Some(_)) | Ok(None) => {}
            Err(err) => {
                warn!("failed to refresh guardian trunk rollout snapshot: {err}");
            }
        }
    }
}

impl<H> EphemeralReviewCleanup<H>
where
    H: GuardianReviewSessionHost,
{
    fn new(
        state: Arc<Mutex<GuardianReviewSessionState<H>>>,
        review_session: Arc<GuardianReviewSession<H>>,
    ) -> Self {
        Self {
            state,
            review_session: Some(review_session),
        }
    }

    fn disarm(&mut self) {
        self.review_session = None;
    }
}

impl<H> Drop for EphemeralReviewCleanup<H>
where
    H: GuardianReviewSessionHost,
{
    fn drop(&mut self) {
        let Some(review_session) = self.review_session.take() else {
            return;
        };
        let state = Arc::clone(&self.state);
        drop(tokio::spawn(async move {
            let review_session = {
                let mut state = state.lock().await;
                state
                    .ephemeral_reviews
                    .iter()
                    .position(|active_review| Arc::ptr_eq(active_review, &review_session))
                    .map(|index| state.ephemeral_reviews.swap_remove(index))
            };
            if let Some(review_session) = review_session {
                review_session.shutdown().await;
            }
        }));
    }
}

impl<H> GuardianReviewSessionManager<H>
where
    H: GuardianReviewSessionHost,
{
    pub async fn trunk_rollout_path(&self) -> Option<PathBuf> {
        let trunk = self.state.lock().await.trunk.clone()?;
        trunk.host.trunk_rollout_path(&trunk.session).await
    }

    pub async fn shutdown(&self) {
        let (review_session, ephemeral_reviews) = {
            let mut state = self.state.lock().await;
            (
                state.trunk.take(),
                std::mem::take(&mut state.ephemeral_reviews),
            )
        };
        if let Some(review_session) = review_session {
            review_session.shutdown().await;
        }
        for review_session in ephemeral_reviews {
            review_session.shutdown().await;
        }
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "review session selection and trunk spawning must stay serialized"
    )]
    pub async fn run_review(
        &self,
        params: GuardianReviewSessionParams<H>,
    ) -> (GuardianReviewSessionOutcome, GuardianReviewAnalyticsResult) {
        let deadline = tokio::time::Instant::now() + GUARDIAN_REVIEW_TIMEOUT;
        let next_reuse_key = params.reuse_key.clone();
        let mut stale_trunk_to_shutdown = None;
        let mut spawned_trunk = false;
        let trunk_candidate = match run_before_review_deadline(
            deadline,
            params.external_cancel.as_ref(),
            self.state.lock(),
        )
        .await
        {
            Ok(mut state) => {
                if let Some(trunk) = state.trunk.as_ref()
                    && trunk.reuse_key != next_reuse_key
                    && trunk.review_lock.try_acquire().is_ok()
                {
                    stale_trunk_to_shutdown = state.trunk.take();
                }

                if state.trunk.is_none() {
                    let spawn_cancel_token = CancellationToken::new();
                    let review_session = match run_before_review_deadline_with_cancel(
                        deadline,
                        params.external_cancel.as_ref(),
                        &spawn_cancel_token,
                        Box::pin(spawn_guardian_review_session(
                            &params,
                            GuardianReviewSpawnKind::Trunk,
                            spawn_cancel_token.clone(),
                            /*fork_snapshot*/ None,
                        )),
                    )
                    .await
                    {
                        Ok(Ok(review_session)) => Arc::new(review_session),
                        Ok(Err(err)) => {
                            return (
                                GuardianReviewSessionOutcome::PromptBuildFailed(err),
                                GuardianReviewAnalyticsResult::without_session(),
                            );
                        }
                        Err(outcome) => {
                            return (outcome, GuardianReviewAnalyticsResult::without_session());
                        }
                    };
                    state.trunk = Some(Arc::clone(&review_session));
                    spawned_trunk = true;
                }

                state.trunk.as_ref().cloned()
            }
            Err(outcome) => return (outcome, GuardianReviewAnalyticsResult::without_session()),
        };

        if let Some(review_session) = stale_trunk_to_shutdown {
            review_session.shutdown_in_background();
        }

        let Some(trunk) = trunk_candidate else {
            return (
                GuardianReviewSessionOutcome::Completed(Err(anyhow!(
                    "guardian review session was not available after spawn"
                ))),
                GuardianReviewAnalyticsResult::without_session(),
            );
        };

        if trunk.reuse_key != next_reuse_key {
            return Box::pin(self.run_ephemeral_review(
                params,
                next_reuse_key,
                deadline,
                /*fork_snapshot*/ None,
            ))
            .await;
        }

        let trunk_guard = match trunk.review_lock.try_acquire() {
            Ok(trunk_guard) => trunk_guard,
            Err(_) => {
                return Box::pin(self.run_ephemeral_review(
                    params,
                    next_reuse_key,
                    deadline,
                    trunk.fork_snapshot().await,
                ))
                .await;
            }
        };

        let guardian_session_kind = if spawned_trunk {
            GuardianReviewSessionKind::TrunkNew
        } else {
            GuardianReviewSessionKind::TrunkReused
        };
        let (outcome, keep_review_session, analytics_result) = Box::pin(run_review_on_session(
            trunk.as_ref(),
            &params,
            guardian_session_kind,
            deadline,
        ))
        .await;
        if keep_review_session && matches!(outcome, GuardianReviewSessionOutcome::Completed(_)) {
            trunk.refresh_last_committed_fork_snapshot().await;
        }
        drop(trunk_guard);

        if keep_review_session {
            (outcome, analytics_result)
        } else {
            if let Some(review_session) = self.remove_trunk_if_current(&trunk).await {
                review_session.shutdown_in_background();
            }
            (outcome, analytics_result)
        }
    }

    #[doc(hidden)]
    pub async fn cache_session_for_test(
        &self,
        host: Arc<H>,
        session: H::Session,
        reuse_key: H::ReuseKey,
    ) {
        self.state.lock().await.trunk = Some(Arc::new(GuardianReviewSession {
            host,
            reuse_key,
            session,
            cancel_token: CancellationToken::new(),
            review_lock: Semaphore::new(/*permits*/ 1),
            state: Mutex::new(GuardianReviewState {
                prior_review_count: 0,
                last_reviewed_transcript_cursor: None,
                last_committed_fork_snapshot: None,
            }),
        }));
    }

    #[doc(hidden)]
    pub async fn register_ephemeral_session_for_test(
        &self,
        host: Arc<H>,
        session: H::Session,
        reuse_key: H::ReuseKey,
    ) {
        self.state
            .lock()
            .await
            .ephemeral_reviews
            .push(Arc::new(GuardianReviewSession {
                host,
                reuse_key,
                session,
                cancel_token: CancellationToken::new(),
                review_lock: Semaphore::new(/*permits*/ 1),
                state: Mutex::new(GuardianReviewState {
                    prior_review_count: 0,
                    last_reviewed_transcript_cursor: None,
                    last_committed_fork_snapshot: None,
                }),
            }));
    }

    #[doc(hidden)]
    pub async fn committed_fork_rollout_items_for_test(&self) -> Option<Vec<RolloutItem>> {
        let trunk = self.state.lock().await.trunk.clone()?;
        let state = trunk.state.lock().await;
        let snapshot = state.last_committed_fork_snapshot.as_ref()?;
        match &snapshot.initial_history {
            InitialHistory::Forked(items) => Some(items.clone()),
            InitialHistory::New | InitialHistory::Cleared | InitialHistory::Resumed(_) => None,
        }
    }

    #[doc(hidden)]
    pub async fn send_trunk_event_raw_for_test(&self, event: Event) {
        let Some(trunk) = self.state.lock().await.trunk.clone() else {
            return;
        };
        trunk
            .host
            .send_event_raw_for_test(&trunk.session, event)
            .await;
    }

    async fn remove_trunk_if_current(
        &self,
        trunk: &Arc<GuardianReviewSession<H>>,
    ) -> Option<Arc<GuardianReviewSession<H>>> {
        let mut state = self.state.lock().await;
        if state
            .trunk
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, trunk))
        {
            state.trunk.take()
        } else {
            None
        }
    }

    async fn register_active_ephemeral(&self, review_session: Arc<GuardianReviewSession<H>>) {
        self.state
            .lock()
            .await
            .ephemeral_reviews
            .push(review_session);
    }

    async fn take_active_ephemeral(
        &self,
        review_session: &Arc<GuardianReviewSession<H>>,
    ) -> Option<Arc<GuardianReviewSession<H>>> {
        let mut state = self.state.lock().await;
        let ephemeral_review_index = state
            .ephemeral_reviews
            .iter()
            .position(|active_review| Arc::ptr_eq(active_review, review_session))?;
        Some(state.ephemeral_reviews.swap_remove(ephemeral_review_index))
    }

    async fn run_ephemeral_review(
        &self,
        params: GuardianReviewSessionParams<H>,
        reuse_key: H::ReuseKey,
        deadline: tokio::time::Instant,
        fork_snapshot: Option<GuardianReviewForkSnapshot>,
    ) -> (GuardianReviewSessionOutcome, GuardianReviewAnalyticsResult) {
        let spawn_cancel_token = CancellationToken::new();
        let review_session = match run_before_review_deadline_with_cancel(
            deadline,
            params.external_cancel.as_ref(),
            &spawn_cancel_token,
            Box::pin(spawn_guardian_review_session(
                &params,
                GuardianReviewSpawnKind::Ephemeral,
                spawn_cancel_token.clone(),
                fork_snapshot,
            )),
        )
        .await
        {
            Ok(Ok(review_session)) => Arc::new(review_session),
            Ok(Err(err)) => {
                return (
                    GuardianReviewSessionOutcome::PromptBuildFailed(err),
                    GuardianReviewAnalyticsResult::without_session(),
                );
            }
            Err(outcome) => return (outcome, GuardianReviewAnalyticsResult::without_session()),
        };
        self.register_active_ephemeral(Arc::clone(&review_session))
            .await;
        let mut cleanup =
            EphemeralReviewCleanup::new(Arc::clone(&self.state), Arc::clone(&review_session));

        let (outcome, _, analytics_result) = Box::pin(run_review_on_session(
            review_session.as_ref(),
            &params,
            GuardianReviewSessionKind::EphemeralForked,
            deadline,
        ))
        .await;
        if let Some(review_session) = self.take_active_ephemeral(&review_session).await {
            cleanup.disarm();
            review_session.shutdown_in_background();
        }
        let _ = reuse_key;
        (outcome, analytics_result)
    }
}

async fn spawn_guardian_review_session<H>(
    params: &GuardianReviewSessionParams<H>,
    spawn_kind: GuardianReviewSpawnKind,
    cancel_token: CancellationToken,
    fork_snapshot: Option<GuardianReviewForkSnapshot>,
) -> anyhow::Result<GuardianReviewSession<H>>
where
    H: GuardianReviewSessionHost,
{
    let prior_review_count = fork_snapshot
        .as_ref()
        .map_or(0, |snapshot| snapshot.prior_review_count);
    let initial_transcript_cursor = fork_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.last_reviewed_transcript_cursor);
    let session = params
        .host
        .spawn_session(
            &params.host_request,
            spawn_kind,
            cancel_token.clone(),
            fork_snapshot,
        )
        .await?;

    Ok(GuardianReviewSession {
        host: Arc::clone(&params.host),
        session,
        cancel_token,
        reuse_key: params.reuse_key.clone(),
        review_lock: Semaphore::new(/*permits*/ 1),
        state: Mutex::new(GuardianReviewState {
            prior_review_count,
            last_reviewed_transcript_cursor: initial_transcript_cursor,
            last_committed_fork_snapshot: None,
        }),
    })
}

async fn run_review_on_session<H>(
    review_session: &GuardianReviewSession<H>,
    params: &GuardianReviewSessionParams<H>,
    guardian_session_kind: GuardianReviewSessionKind,
    deadline: tokio::time::Instant,
) -> (
    GuardianReviewSessionOutcome,
    bool,
    GuardianReviewAnalyticsResult,
)
where
    H: GuardianReviewSessionHost,
{
    let (send_followup_reminder, prompt_mode) = {
        let state = review_session.state.lock().await;

        let send_followup_reminder = state.prior_review_count == 1;
        let prompt_mode = if state.prior_review_count == 0 {
            GuardianPromptMode::Full
        } else if let Some(cursor) = state.last_reviewed_transcript_cursor {
            GuardianPromptMode::Delta { cursor }
        } else {
            GuardianPromptMode::Full
        };

        (send_followup_reminder, prompt_mode)
    };
    let model_info = params.host.model_info(&params.host_request).await;
    let guardian_reasoning_effort = if model_info.supports_reasoning_summaries {
        params
            .reasoning_effort
            .or(model_info.default_reasoning_level)
    } else {
        None
    };
    let mut analytics_result = GuardianReviewAnalyticsResult::from_session(
        review_session.host.session_id(&review_session.session),
        guardian_session_kind,
        params.model.clone(),
        guardian_reasoning_effort.map(|effort| effort.to_string()),
        had_prior_review_context(&prompt_mode),
    );
    if send_followup_reminder {
        review_session
            .host
            .append_followup_reminder(&review_session.session)
            .await;
    }

    let prompt_items = run_before_review_deadline(
        deadline,
        params.external_cancel.as_ref(),
        Box::pin(async {
            review_session
                .host
                .sync_network_approval(&params.host_request, &review_session.session)
                .await;

            review_session
                .host
                .build_prompt_items(
                    &params.host_request,
                    params.retry_reason.clone(),
                    params.request.clone(),
                    prompt_mode,
                )
                .await
        }),
    )
    .await;
    let prompt_items = match prompt_items {
        Ok(prompt_items) => prompt_items,
        Err(outcome) => return (outcome, false, analytics_result),
    };
    let prompt_items = match prompt_items {
        Ok(prompt_items) => prompt_items,
        Err(err) => {
            return (
                GuardianReviewSessionOutcome::PromptBuildFailed(err.into()),
                false,
                analytics_result,
            );
        }
    };
    let reviewed_action_truncated = prompt_items.reviewed_action_truncated;
    let transcript_cursor = prompt_items.transcript_cursor;
    let token_usage_at_review_start = review_session
        .host
        .total_token_usage(&review_session.session)
        .await
        .unwrap_or_default();

    let submit_result = run_before_review_deadline(
        deadline,
        params.external_cancel.as_ref(),
        Box::pin(review_session.host.submit_review(
            &review_session.session,
            &params.host_request,
            prompt_items.items,
            params.schema.clone(),
            params.model.clone(),
            params.reasoning_effort,
            params.reasoning_summary,
            params.personality,
        )),
    )
    .await;
    let child_turn_id = match submit_result {
        Ok(Ok(child_turn_id)) => child_turn_id,
        Ok(Err(err)) => {
            return (
                GuardianReviewSessionOutcome::SessionFailed(err),
                false,
                analytics_result,
            );
        }
        Err(outcome) => return (outcome, false, analytics_result),
    };
    analytics_result.reviewed_action_truncated = reviewed_action_truncated;

    let outcome = wait_for_guardian_review(
        review_session,
        child_turn_id.as_str(),
        deadline,
        params.external_cancel.as_ref(),
        &mut analytics_result,
    )
    .await;
    if matches!(outcome.0, GuardianReviewSessionOutcome::Completed(_)) {
        if outcome.2
            && let Some(total_token_usage) = review_session
                .host
                .total_token_usage(&review_session.session)
                .await
        {
            analytics_result.token_usage = Some(token_usage_delta(
                &token_usage_at_review_start,
                &total_token_usage,
            ));
        }
        let mut state = review_session.state.lock().await;
        state.prior_review_count = state.prior_review_count.saturating_add(1);
        state.last_reviewed_transcript_cursor = Some(transcript_cursor);
    }
    (outcome.0, outcome.1, analytics_result)
}

async fn wait_for_guardian_review<H>(
    review_session: &GuardianReviewSession<H>,
    expected_turn_id: &str,
    deadline: tokio::time::Instant,
    external_cancel: Option<&CancellationToken>,
    analytics_result: &mut GuardianReviewAnalyticsResult,
) -> (GuardianReviewSessionOutcome, bool, bool)
where
    H: GuardianReviewSessionHost,
{
    let timeout = tokio::time::sleep_until(deadline);
    tokio::pin!(timeout);
    let mut last_error_message: Option<String> = None;

    loop {
        tokio::select! {
            _ = &mut timeout => {
                let keep_review_session = review_session
                    .host
                    .interrupt_and_drain(
                        &review_session.session,
                        expected_turn_id,
                        GUARDIAN_INTERRUPT_DRAIN_TIMEOUT,
                    )
                    .await
                    .is_ok();
                return (GuardianReviewSessionOutcome::TimedOut, keep_review_session, false);
            }
            _ = async {
                if let Some(cancel_token) = external_cancel {
                    cancel_token.cancelled().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {
                let keep_review_session = review_session
                    .host
                    .interrupt_and_drain(
                        &review_session.session,
                        expected_turn_id,
                        GUARDIAN_INTERRUPT_DRAIN_TIMEOUT,
                    )
                    .await
                    .is_ok();
                return (GuardianReviewSessionOutcome::Aborted, keep_review_session, false);
            }
            event = review_session.host.next_event(&review_session.session) => {
                match event {
                    Ok(event) if !event_matches_turn(&event, expected_turn_id) => {}
                    Ok(event) => match event.msg {
                        EventMsg::TurnComplete(turn_complete) => {
                            analytics_result.time_to_first_token_ms = turn_complete
                                .time_to_first_token_ms
                                .and_then(|ms| u64::try_from(ms).ok());
                            if turn_complete.last_agent_message.is_none()
                                && let Some(error_message) = last_error_message
                            {
                                return (
                                    GuardianReviewSessionOutcome::Completed(Err(anyhow!(error_message))),
                                    true,
                                    true,
                                );
                            }
                            return (
                                GuardianReviewSessionOutcome::Completed(Ok(turn_complete.last_agent_message)),
                                true,
                                true,
                            );
                        }
                        EventMsg::Error(error) => {
                            last_error_message = Some(error.message);
                        }
                        EventMsg::TurnAborted(_) => {
                            return (GuardianReviewSessionOutcome::Aborted, true, false);
                        }
                        _ => {}
                    },
                    Err(err) => {
                        return (
                            GuardianReviewSessionOutcome::Completed(Err(err)),
                            false,
                            false,
                        );
                    }
                }
            }
        }
    }
}

fn event_matches_turn(event: &Event, expected_turn_id: &str) -> bool {
    if event.id != expected_turn_id {
        return false;
    }

    match &event.msg {
        EventMsg::TurnComplete(turn_complete) => turn_complete.turn_id == expected_turn_id,
        EventMsg::TurnAborted(turn_aborted) => {
            turn_aborted.turn_id.as_deref() == Some(expected_turn_id)
        }
        _ => true,
    }
}

async fn run_before_review_deadline<T>(
    deadline: tokio::time::Instant,
    external_cancel: Option<&CancellationToken>,
    future: impl Future<Output = T>,
) -> Result<T, GuardianReviewSessionOutcome> {
    tokio::select! {
        _ = tokio::time::sleep_until(deadline) => Err(GuardianReviewSessionOutcome::TimedOut),
        result = future => Ok(result),
        _ = async {
            if let Some(cancel_token) = external_cancel {
                cancel_token.cancelled().await;
            } else {
                std::future::pending::<()>().await;
            }
        } => Err(GuardianReviewSessionOutcome::Aborted),
    }
}

async fn run_before_review_deadline_with_cancel<T>(
    deadline: tokio::time::Instant,
    external_cancel: Option<&CancellationToken>,
    cancel_token: &CancellationToken,
    future: impl Future<Output = T>,
) -> Result<T, GuardianReviewSessionOutcome> {
    let result = run_before_review_deadline(deadline, external_cancel, future).await;
    if result.is_err() {
        cancel_token.cancel();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn run_before_review_deadline_times_out_before_future_completes() {
        let outcome = run_before_review_deadline(
            tokio::time::Instant::now() + Duration::from_millis(10),
            /*external_cancel*/ None,
            async {
                tokio::time::sleep(Duration::from_millis(50)).await;
            },
        )
        .await;

        assert!(matches!(
            outcome,
            Err(GuardianReviewSessionOutcome::TimedOut)
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_before_review_deadline_aborts_when_cancelled() {
        let cancel_token = CancellationToken::new();
        let canceller = cancel_token.clone();
        drop(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            canceller.cancel();
        }));

        let outcome = run_before_review_deadline(
            tokio::time::Instant::now() + Duration::from_secs(1),
            Some(&cancel_token),
            std::future::pending::<()>(),
        )
        .await;

        assert!(matches!(
            outcome,
            Err(GuardianReviewSessionOutcome::Aborted)
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_before_review_deadline_with_cancel_cancels_token_on_timeout() {
        let cancel_token = CancellationToken::new();

        let outcome = run_before_review_deadline_with_cancel(
            tokio::time::Instant::now() + Duration::from_millis(10),
            /*external_cancel*/ None,
            &cancel_token,
            async {
                tokio::time::sleep(Duration::from_millis(50)).await;
            },
        )
        .await;

        assert!(matches!(
            outcome,
            Err(GuardianReviewSessionOutcome::TimedOut)
        ));
        assert!(cancel_token.is_cancelled());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_before_review_deadline_with_cancel_cancels_token_on_abort() {
        let external_cancel = CancellationToken::new();
        let external_canceller = external_cancel.clone();
        let cancel_token = CancellationToken::new();
        drop(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            external_canceller.cancel();
        }));

        let outcome = run_before_review_deadline_with_cancel(
            tokio::time::Instant::now() + Duration::from_secs(1),
            Some(&external_cancel),
            &cancel_token,
            std::future::pending::<()>(),
        )
        .await;

        assert!(matches!(
            outcome,
            Err(GuardianReviewSessionOutcome::Aborted)
        ));
        assert!(cancel_token.is_cancelled());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_before_review_deadline_with_cancel_preserves_token_on_success() {
        let cancel_token = CancellationToken::new();

        let outcome = run_before_review_deadline_with_cancel(
            tokio::time::Instant::now() + Duration::from_secs(1),
            /*external_cancel*/ None,
            &cancel_token,
            async { 42usize },
        )
        .await;

        assert_eq!(outcome.unwrap(), 42);
        assert!(!cancel_token.is_cancelled());
    }

    #[test]
    fn had_prior_review_context_tracks_prompt_mode() {
        assert!(!had_prior_review_context(&GuardianPromptMode::Full));
        assert!(had_prior_review_context(&GuardianPromptMode::Delta {
            cursor: GuardianTranscriptCursor {
                parent_history_version: 7,
                transcript_entry_count: 42,
            }
        }));
    }

    #[test]
    fn token_usage_delta_never_reports_negative_usage() {
        let start = TokenUsage {
            input_tokens: 10,
            cached_input_tokens: 8,
            output_tokens: 6,
            reasoning_output_tokens: 4,
            total_tokens: 28,
        };
        let end = TokenUsage {
            input_tokens: 15,
            cached_input_tokens: 7,
            output_tokens: 10,
            reasoning_output_tokens: 2,
            total_tokens: 34,
        };

        assert_eq!(
            token_usage_delta(&start, &end),
            TokenUsage {
                input_tokens: 5,
                cached_input_tokens: 0,
                output_tokens: 4,
                reasoning_output_tokens: 0,
                total_tokens: 6,
            }
        );
    }
}
